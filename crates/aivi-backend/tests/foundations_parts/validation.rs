#[test]
fn lowering_makes_local_bindings_explicit_environment_slots() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::Local(
        HirBindingId::from_raw(7),
    )));
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    let item = find_item(&backend, "captured");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
    let BackendStageKind::Gate(BackendGateStage::Ordinary {
        when_true,
        when_false,
    }) = &pipeline.stages[0].kind
    else {
        panic!("expected ordinary gate stage");
    };
    assert_eq!(backend.kernels()[*when_true].environment.len(), 1);
    assert_eq!(backend.kernels()[*when_true].input_subject, None);
    assert!(backend.kernels()[*when_false].environment.is_empty());
}

#[test]
fn lowers_direct_signal_dependencies_into_dedicated_body_kernels() {
    let backend = lower_text(
        "backend-direct-signal-body-kernel.aivi",
        r#"
signal base = 1
signal derived = base
"#,
    );
    let base = find_item(&backend, "base");
    let derived = find_item(&backend, "derived");
    let derived_item = &backend.items()[derived];
    let BackendItemKind::Signal(signal) = &derived_item.kind else {
        panic!("derived should remain a signal item");
    };

    assert_eq!(signal.dependencies, vec![base]);
    assert_eq!(signal.dependency_layouts.len(), 1);

    let body_kernel_id = signal
        .body_kernel
        .expect("direct dependency signal should lower a dedicated body kernel");
    let body_kernel = &backend.kernels()[body_kernel_id];
    assert!(
        matches!(body_kernel.origin.kind, KernelOriginKind::SignalBody { item } if item == derived)
    );
    assert_eq!(body_kernel.input_subject, None);
    assert_eq!(body_kernel.environment, signal.dependency_layouts);
    assert!(body_kernel.global_items.is_empty());
    assert!(matches!(
        body_kernel.exprs()[body_kernel.root].kind,
        KernelExprKind::Environment(_)
    ));

    let fallback_kernel_id = derived_item
        .body
        .expect("signal items should keep their fallback body kernel");
    let fallback_kernel = &backend.kernels()[fallback_kernel_id];
    assert!(fallback_kernel.global_items.contains(&base));
    assert!(matches!(
        fallback_kernel.exprs()[fallback_kernel.root].kind,
        KernelExprKind::Item(item) if item == base
    ));
}

#[test]
fn validator_rejects_signal_body_kernels_that_still_take_input() {
    let mut backend = lower_text(
        "backend-direct-signal-body-kernel-invalid.aivi",
        r#"
signal base = 1
signal derived = base
"#,
    );
    let derived = find_item(&backend, "derived");
    let (body_kernel, dependency_layout) = match &backend.items()[derived].kind {
        BackendItemKind::Signal(signal) => (
            signal
                .body_kernel
                .expect("direct dependency signal should lower a dedicated body kernel"),
            signal.dependency_layouts[0],
        ),
        other => panic!("expected signal item, found {other:?}"),
    };
    backend
        .kernels_mut()
        .get_mut(body_kernel)
        .expect("signal body kernel should exist")
        .input_subject = Some(dependency_layout);

    let errors = validate_program(&backend)
        .expect_err("signal body kernels with an input subject should fail validation");
    assert!(errors.errors().iter().any(|error| {
        matches!(
            error,
            ValidationError::SignalBodyHasInput { item, kernel, layout }
                if *item == derived && *kernel == body_kernel && *layout == dependency_layout
        )
    }));
}

#[test]
fn validator_catches_missing_kernel_input_subject() {
    let mut backend = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
    let maybe_active = find_item(&backend, "maybeActive");
    let pipeline_id = first_pipeline(&backend, maybe_active);
    let when_true = match &backend.pipelines()[pipeline_id].stages[0].kind {
        BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) => *when_true,
        other => panic!("expected ordinary gate stage, found {other:?}"),
    };
    backend
        .kernels_mut()
        .get_mut(when_true)
        .expect("gate kernel should exist")
        .input_subject = None;

    let errors =
        validate_program(&backend).expect_err("missing input subject should fail validation");
    assert!(errors.errors().iter().any(|error| {
        matches!(
            error,
            ValidationError::KernelMissingInputSubject { kernel, .. }
                | ValidationError::KernelConventionMismatch { kernel }
                if *kernel == when_true
        )
    }));
}

#[test]
fn lowering_rejects_unresolved_hir_item_references() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::HirItem(
        HirItemId::from_raw(99),
    )));
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    let errors =
        lower_backend_module(&lambda).expect_err("unresolved HIR item reference should fail");
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| matches!(error, LoweringError::UnresolvedItemReference { .. }))
    );
}

#[test]
fn lowering_rejects_unsupported_local_references_in_source_kernels() {
    let core = manual_core_source_argument_signal(
        CoreType::Primitive(BuiltinType::Int),
        CoreExprKind::Reference(CoreReference::Local(HirBindingId::from_raw(7))),
    );
    validate_core_module(&core).expect("manual source module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let errors = lower_backend_module(&lambda)
        .expect_err("source runtime locals should fail backend lowering");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::UnsupportedLocalReference { binding, .. } if *binding == 7
    )));
}

#[test]
fn lowering_maps_open_type_parameters_to_erased_domain_layouts() {
    let open = CoreType::TypeParameter {
        parameter: HirTypeParameterId::from_raw(0),
        name: "a".into(),
    };
    let core = manual_core_gate_stage(
        open.clone(),
        open.clone(),
        {
            let open = open.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: open.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
        {
            let open = open.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: open.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual open-type module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda)
        .expect("open type parameters should lower via erased Domain layouts");
    // The open type parameter `a` maps to a Domain layout so the kernel can be compiled;
    // call sites are monomorphic by the time they reach the runtime.
    let has_domain_a = backend.layouts().iter().any(|(_, layout)| {
        matches!(&layout.kind, LayoutKind::Domain { name, .. } if name.as_ref() == "a")
    });
    assert!(
        has_domain_a,
        "open type parameter should be lowered to an erased Domain layout"
    );
}

#[test]
fn lowering_rejects_global_item_cycles() {
    let span = SourceSpan::default();
    let mut core = CoreModule::new();
    let left = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "left".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("left item allocation should fit");
    let right = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(1),
            span,
            name: "right".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("right item allocation should fit");
    let left_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Reference(CoreReference::Item(right)),
        })
        .expect("left body allocation should fit");
    let right_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Reference(CoreReference::Item(left)),
        })
        .expect("right body allocation should fit");
    core.items_mut()
        .get_mut(left)
        .expect("left item should exist")
        .body = Some(left_body);
    core.items_mut()
        .get_mut(right)
        .expect("right item should exist")
        .body = Some(right_body);

    validate_core_module(&core).expect("manual cyclic module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let errors =
        lower_backend_module(&lambda).expect_err("global item cycles should fail lowering");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::GlobalItemCycle { item } if *item == aivi_backend::ItemId::from_raw(0)
            || *item == aivi_backend::ItemId::from_raw(1)
    )));
}

