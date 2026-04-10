struct ModuleLowerer<'a> {
    hir: &'a aivi_hir::Module,
    included_items: Option<HashSet<HirItemId>>,
    debug_items: HashSet<HirItemId>,
    mock_overrides: HashMap<HirItemId, HashMap<ImportId, MockImportTarget>>,
    module: Module,
    item_map: HashMap<HirItemId, ItemId>,
    import_item_map: HashMap<ImportId, ItemId>,
    domain_member_item_map: HashMap<DomainMemberKey, ItemId>,
    instance_member_item_map: HashMap<InstanceMemberKey, ItemId>,
    pipe_builders: BTreeMap<PipeKey, PipeBuilder>,
    source_by_owner: HashMap<ItemId, SourceId>,
    decode_by_owner: HashMap<ItemId, DecodeProgramId>,
    next_synthetic_item_origin_raw: u32,
    next_synthetic_binding_raw: u32,
    // Base index of the HIR item arena; used to assign deterministic synthetic origins
    // to signal imports so that both this lowerer and the runtime assembly builder
    // independently derive the same HirItemId for the same import.
    hir_item_count: u32,
    errors: Vec<LoweringError>,
    // Pre-compiled workspace module items: module_name → (exported_name → core ItemId).
    // Populated by compile_workspace_module before entry module compilation in aivi run.
    workspace_name_maps: HashMap<Box<str>, HashMap<Box<str>, ItemId>>,
    // Pre-compiled workspace sum constructors: module_name → (constructor_name → parent type
    // origin). Used so imported constructor values and imported workspace function bodies share
    // the same runtime constructor identity.
    workspace_constructor_origins: HashMap<Box<str>, HashMap<Box<str>, HirItemId>>,
    // Maps ImportId → module name string for the currently active HIR.
    // Rebuilt whenever self.hir changes (entry module or a workspace module).
    import_to_module: HashMap<ImportId, Box<str>>,
    // Per-module offset added to HIR item IDs when assigning origins in seed_items and
    // seed_instance_member_item. Zero for the entry module; set to a workspace-module-specific
    // base when compiling workspace modules to prevent origin collisions across modules.
    item_origin_offset: u32,
    // Monotonically advancing base for workspace module origin space assignments.
    // NOT saved/restored in compile_workspace_module — persists across all workspace modules
    // so each module receives a non-overlapping origin range.
    ws_origin_base: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MockImportTarget {
    Item(HirItemId),
    Import(ImportId),
}

fn is_markup_value(hir: &aivi_hir::Module, item_id: HirItemId) -> bool {
    matches!(
        hir.items().get(item_id),
        Some(HirItem::Value(value))
            if matches!(hir.exprs()[value.body].kind, HirExprKind::Markup(_))
    )
}

fn collect_debug_items(
    hir: &aivi_hir::Module,
    included_items: Option<&HashSet<HirItemId>>,
) -> HashSet<HirItemId> {
    hir.items()
        .iter()
        .filter(|(item_id, _)| included_items.is_none_or(|included| included.contains(item_id)))
        .filter_map(|(item_id, item)| {
            item.decorators()
                .iter()
                .any(|decorator_id| {
                    hir.decorators()
                        .get(*decorator_id)
                        .is_some_and(|decorator| {
                            matches!(decorator.payload, DecoratorPayload::Debug(_))
                        })
                })
                .then_some(item_id)
        })
        .collect()
}

fn collect_mock_overrides(
    hir: &aivi_hir::Module,
    included_items: Option<&HashSet<HirItemId>>,
) -> HashMap<HirItemId, HashMap<ImportId, MockImportTarget>> {
    let mut overrides = HashMap::new();
    for (item_id, item) in hir.items().iter() {
        if included_items.is_some_and(|included| !included.contains(&item_id)) {
            continue;
        }
        let mut item_overrides = HashMap::new();
        for decorator_id in item.decorators() {
            let Some(decorator) = hir.decorators().get(*decorator_id) else {
                continue;
            };
            let DecoratorPayload::Mock(mock) = &decorator.payload else {
                continue;
            };
            let Some(target_import) = mock_target_import(hir, mock.target) else {
                continue;
            };
            let Some(target) = mock_replacement_target(hir, mock.replacement) else {
                continue;
            };
            item_overrides.insert(target_import, target);
        }
        if !item_overrides.is_empty() {
            overrides.insert(item_id, item_overrides);
        }
    }
    overrides
}

fn mock_target_import(hir: &aivi_hir::Module, expr_id: HirExprId) -> Option<ImportId> {
    let HirExprKind::Name(reference) = &hir.exprs().get(expr_id)?.kind else {
        return None;
    };
    let aivi_hir::ResolutionState::Resolved(TermResolution::Import(import_id)) =
        reference.resolution
    else {
        return None;
    };
    Some(import_id)
}

fn mock_replacement_target(hir: &aivi_hir::Module, expr_id: HirExprId) -> Option<MockImportTarget> {
    let HirExprKind::Name(reference) = &hir.exprs().get(expr_id)?.kind else {
        return None;
    };
    match reference.resolution {
        aivi_hir::ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            Some(MockImportTarget::Item(item_id))
        }
        aivi_hir::ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            Some(MockImportTarget::Import(import_id))
        }
        _ => None,
    }
}

struct RuntimeFragmentLowerer<'a> {
    lowerer: ModuleLowerer<'a>,
    fragment: &'a RuntimeFragmentSpec,
    report_by_owner: HashMap<HirItemId, aivi_hir::GeneralExprItemElaboration>,
    domain_member_reports: HashMap<DomainMemberKey, aivi_hir::GeneralExprDomainMemberElaboration>,
    instance_member_reports: HashMap<InstanceMemberKey, GeneralExprInstanceMemberElaboration>,
    lowering: HashSet<HirItemId>,
    lowered: HashSet<HirItemId>,
    lowering_domain_members: HashSet<DomainMemberKey>,
    lowered_domain_members: HashSet<DomainMemberKey>,
    lowering_instance_members: HashSet<InstanceMemberKey>,
    lowered_instance_members: HashSet<InstanceMemberKey>,
}

struct RuntimeFragmentItemCollector<'a> {
    hir: &'a aivi_hir::Module,
    fragment: &'a RuntimeFragmentSpec,
    report_by_owner: HashMap<HirItemId, aivi_hir::GeneralExprItemElaboration>,
    domain_member_reports: HashMap<DomainMemberKey, aivi_hir::GeneralExprDomainMemberElaboration>,
    instance_member_reports: HashMap<InstanceMemberKey, GeneralExprInstanceMemberElaboration>,
    included_items: HashSet<HirItemId>,
    visited_domain_members: HashSet<DomainMemberKey>,
    visited_instance_members: HashSet<InstanceMemberKey>,
}

impl<'a> ModuleLowerer<'a> {
    fn new(hir: &'a aivi_hir::Module) -> Self {
        Self::new_internal(hir, None)
    }

    fn new_with_items(hir: &'a aivi_hir::Module, included_items: &HashSet<HirItemId>) -> Self {
        Self::new_internal(hir, Some(included_items.clone()))
    }

    fn new_runtime(hir: &'a aivi_hir::Module) -> Self {
        let included_items = hir
            .items()
            .iter()
            .filter_map(|(item_id, item)| match item {
                HirItem::Value(value)
                    if matches!(hir.exprs()[value.body].kind, aivi_hir::ExprKind::Markup(_)) =>
                {
                    None
                }
                _ => Some(item_id),
            })
            .collect::<HashSet<_>>();
        Self::new_internal(hir, Some(included_items))
    }

    fn new_runtime_with_items(
        hir: &'a aivi_hir::Module,
        included_items: &HashSet<HirItemId>,
    ) -> Self {
        let included_items = included_items
            .iter()
            .copied()
            .filter(|item_id| !is_markup_value(hir, *item_id))
            .collect::<HashSet<_>>();
        Self::new_internal(hir, Some(included_items))
    }

    /// Build a map of ImportId → module-name-string from a HIR's UseItems.
    /// Used so that seed_import_item can look up pre-compiled workspace items.
    fn make_import_to_module_map(hir: &aivi_hir::Module) -> HashMap<ImportId, Box<str>> {
        let mut map = HashMap::new();
        for (_, item) in hir.items().iter() {
            let HirItem::Use(use_item) = item else {
                continue;
            };
            let module_name: Box<str> = use_item.module.to_string().into();
            for import_id in use_item.imports.iter().copied() {
                map.insert(import_id, module_name.clone());
            }
        }
        map
    }

    /// Compile a workspace module into the shared typed-core arena, then save its
    /// name → ItemId map so that entry-module imports can resolve to the real items.
    ///
    /// The method saves and restores all HIR-specific state so that entry-module
    /// compilation is unaffected.  Only `self.module` (the shared item arena),
    /// `self.workspace_name_maps`, and `self.ws_origin_base` grow permanently.
    fn compile_workspace_module(
        &mut self,
        module_name: &str,
        ws_hir: &'a aivi_hir::Module,
    ) -> Result<(), LoweringErrors> {
        // ── Claim a non-overlapping origin slice for this workspace module ────
        // ws_origin_base is persistent (not saved/restored); each workspace module
        // receives a unique origin range so its items never collide with entry-module
        // items (which use raw HIR item IDs 0..hir_item_count) or with items from
        // other workspace modules.
        let module_origin_base = self.ws_origin_base;

        // ── Save all HIR-specific state ──────────────────────────────────────
        let saved_hir = self.hir;
        let saved_included_items = self.included_items.take();
        let saved_item_map = std::mem::take(&mut self.item_map);
        let saved_import_item_map = std::mem::take(&mut self.import_item_map);
        let saved_domain_member_item_map = std::mem::take(&mut self.domain_member_item_map);
        let saved_instance_member_item_map = std::mem::take(&mut self.instance_member_item_map);
        let saved_pipe_builders = std::mem::take(&mut self.pipe_builders);
        let saved_source_by_owner = std::mem::take(&mut self.source_by_owner);
        let saved_decode_by_owner = std::mem::take(&mut self.decode_by_owner);
        let saved_import_to_module = std::mem::take(&mut self.import_to_module);
        let saved_debug_items = std::mem::take(&mut self.debug_items);
        let saved_mock_overrides = std::mem::take(&mut self.mock_overrides);
        let saved_hir_item_count = self.hir_item_count;
        let saved_next_synthetic = self.next_synthetic_item_origin_raw;
        let saved_next_binding = self.next_synthetic_binding_raw;
        let saved_item_origin_offset = self.item_origin_offset;

        // ── Set up for workspace module ──────────────────────────────────────
        let ws_item_count =
            u32::try_from(ws_hir.items().iter().count()).expect("HIR item count fits u32");
        let ws_import_count =
            u32::try_from(ws_hir.imports().iter().count()).expect("HIR import count fits u32");
        let ws_binding_count =
            u32::try_from(ws_hir.bindings().iter().count()).expect("HIR binding count fits u32");

        self.hir = ws_hir;
        let ws_non_markup: HashSet<HirItemId> = ws_hir
            .items()
            .iter()
            .filter(|(item_id, _)| !is_markup_value(ws_hir, *item_id))
            .map(|(item_id, _)| item_id)
            .collect();
        self.included_items = Some(ws_non_markup);
        self.item_map = HashMap::new();
        self.import_item_map = HashMap::new();
        self.domain_member_item_map = HashMap::new();
        self.instance_member_item_map = HashMap::new();
        self.pipe_builders = BTreeMap::new();
        self.source_by_owner = HashMap::new();
        self.decode_by_owner = HashMap::new();
        self.debug_items = collect_debug_items(ws_hir, self.included_items.as_ref());
        self.mock_overrides = collect_mock_overrides(ws_hir, self.included_items.as_ref());
        self.hir_item_count = ws_item_count;
        // Seed items for this workspace module use origins starting at module_origin_base.
        // Synthetic items (domain members, non-signal imports) follow immediately after
        // the reserved [module_origin_base, module_origin_base + ws_item_count + ws_import_count) slice.
        self.item_origin_offset = module_origin_base;
        self.next_synthetic_item_origin_raw = module_origin_base + ws_item_count + ws_import_count;
        self.next_synthetic_binding_raw = ws_binding_count;
        self.import_to_module = Self::make_import_to_module_map(ws_hir);

        // ── Compile workspace module items ───────────────────────────────────
        self.seed_items()?;
        self.lower_general_exprs();

        // ── Advance ws_origin_base past everything this module may have used ─
        // next_synthetic_item_origin_raw now points to the high-water mark of all
        // origins consumed by this module (real items, signal stubs, synthetics).
        self.ws_origin_base = self.next_synthetic_item_origin_raw;

        // ── Save name → ItemId map for this workspace module ─────────────────
        let mut name_map: HashMap<Box<str>, ItemId> = ws_hir
            .items()
            .iter()
            .filter_map(|(hir_id, item)| {
                let name: Box<str> = match item {
                    HirItem::Value(v) => v.name.text().into(),
                    HirItem::Function(f) => f.name.text().into(),
                    HirItem::Signal(s) => s.name.text().into(),
                    _ => return None,
                };
                let core_id = self.item_map.get(&hir_id).copied()?;
                Some((name, core_id))
            })
            .collect();
        // Also include all non-signal imports so that downstream modules that import through
        // a re-export shim resolve to the original implementation item (with body) rather than
        // a bodyless stub. Two passes: first from import_item_map (already-resolved imports from
        // body lowering), then a direct workspace lookup for imports that body lowering never
        // touched (e.g., shim modules with no function bodies where all items are just imports).
        // Signal imports are excluded: they use deterministic synthetic origins that must not
        // be remapped.
        for (import_id, &core_id) in &self.import_item_map {
            if let Some(binding) = ws_hir.imports().get(*import_id)
                && !matches!(self.module.items()[core_id].kind, ItemKind::Signal(_)) {
                    let local_name: Box<str> = binding.local_name.text().into();
                    name_map.entry(local_name).or_insert(core_id);
                }
        }
        // Second pass: directly resolve imports not yet in import_item_map by looking up the
        // workspace name maps of their source modules. This handles pure re-export shims where
        // lower_general_exprs never calls seed_import_item because there are no function bodies.
        for (import_id, binding) in ws_hir.imports().iter() {
            let local_name: Box<str> = binding.local_name.text().into();
            if name_map.contains_key(&local_name) {
                continue;
            }
            // Skip signal imports — they must use deterministic synthetic origin arithmetic and
            // must not be remapped to the workspace item's origin.
            let is_signal = matches!(
                &binding.metadata,
                ImportBindingMetadata::Value {
                    ty: ImportValueType::Signal(_)
                }
            );
            if is_signal {
                continue;
            }
            if let Some(source_module) = self.import_to_module.get(&import_id)
                && let Some(source_name_map) = self.workspace_name_maps.get(source_module.as_ref())
                    && let Some(&core_id) = source_name_map.get(binding.imported_name.text()) {
                        name_map.insert(local_name, core_id);
                    }
        }
        self.workspace_name_maps
            .insert(module_name.into(), name_map);

        let mut constructor_origin_map: HashMap<Box<str>, HirItemId> = HashMap::new();
        for (hir_id, item) in ws_hir.items().iter() {
            let HirItem::Type(type_item) = item else {
                continue;
            };
            let TypeItemBody::Sum(variants) = &type_item.body else {
                continue;
            };
            let origin = HirItemId::from_raw(hir_id.as_raw().saturating_add(module_origin_base));
            for variant in variants.iter() {
                constructor_origin_map
                    .entry(variant.name.text().into())
                    .or_insert(origin);
            }
        }
        for (import_id, binding) in ws_hir.imports().iter() {
            let local_name: Box<str> = binding.local_name.text().into();
            if constructor_origin_map.contains_key(&local_name) {
                continue;
            }
            let Some(source_module) = binding.source_module.as_deref().or_else(|| {
                self.import_to_module
                    .get(&import_id)
                    .map(|name| name.as_ref())
            }) else {
                continue;
            };
            let Some(source_map) = self.workspace_constructor_origins.get(source_module) else {
                continue;
            };
            let Some(&origin) = source_map.get(binding.imported_name.text()) else {
                continue;
            };
            constructor_origin_map.insert(local_name, origin);
        }
        self.workspace_constructor_origins
            .insert(module_name.into(), constructor_origin_map);

        // ── Restore entry module state ───────────────────────────────────────
        self.hir = saved_hir;
        self.included_items = saved_included_items;
        self.item_map = saved_item_map;
        self.import_item_map = saved_import_item_map;
        self.domain_member_item_map = saved_domain_member_item_map;
        self.instance_member_item_map = saved_instance_member_item_map;
        self.pipe_builders = saved_pipe_builders;
        self.source_by_owner = saved_source_by_owner;
        self.decode_by_owner = saved_decode_by_owner;
        self.import_to_module = saved_import_to_module;
        self.debug_items = saved_debug_items;
        self.mock_overrides = saved_mock_overrides;
        self.hir_item_count = saved_hir_item_count;
        self.next_synthetic_item_origin_raw = saved_next_synthetic;
        self.next_synthetic_binding_raw = saved_next_binding;
        self.item_origin_offset = saved_item_origin_offset;
        // NOTE: ws_origin_base is intentionally NOT restored — it must persist.

        Ok(())
    }

    fn new_internal(hir: &'a aivi_hir::Module, included_items: Option<HashSet<HirItemId>>) -> Self {
        let debug_items = collect_debug_items(hir, included_items.as_ref());
        let mock_overrides = collect_mock_overrides(hir, included_items.as_ref());
        let hir_item_count =
            u32::try_from(hir.items().iter().count()).expect("HIR item count should fit in u32");
        let hir_import_count = u32::try_from(hir.imports().iter().count())
            .expect("HIR import count should fit in u32");
        // Reserve [hir_item_count, hir_item_count + hir_import_count) for deterministic
        // signal import origins (one slot per import by ImportId). The sequential counter
        // for all other synthetic items starts after this reserved range.
        let next_synthetic_item_origin_raw = hir_item_count + hir_import_count;
        let next_synthetic_binding_raw = u32::try_from(hir.bindings().iter().count())
            .expect("HIR binding count should fit in u32");
        Self {
            hir,
            included_items,
            debug_items,
            mock_overrides,
            module: Module::new(),
            item_map: HashMap::new(),
            import_item_map: HashMap::new(),
            domain_member_item_map: HashMap::new(),
            instance_member_item_map: HashMap::new(),
            pipe_builders: BTreeMap::new(),
            source_by_owner: HashMap::new(),
            decode_by_owner: HashMap::new(),
            next_synthetic_item_origin_raw,
            next_synthetic_binding_raw,
            hir_item_count,
            errors: Vec::new(),
            workspace_name_maps: HashMap::new(),
            workspace_constructor_origins: HashMap::new(),
            import_to_module: HashMap::new(),
            item_origin_offset: 0,
            ws_origin_base: 0,
        }
    }

    fn includes_item(&self, item: HirItemId) -> bool {
        self.included_items
            .as_ref()
            .is_none_or(|included| included.contains(&item))
    }

    fn debug_label(&self, owner: HirItemId, stage: Option<usize>) -> Box<str> {
        let owner_name = match self.hir.items().get(owner) {
            Some(HirItem::Value(item)) => item.name.text(),
            Some(HirItem::Function(item)) => item.name.text(),
            Some(HirItem::Signal(item)) => item.name.text(),
            Some(HirItem::Type(item)) => item.name.text(),
            Some(HirItem::Class(item)) => item.name.text(),
            Some(HirItem::Domain(item)) => item.name.text(),
            Some(HirItem::SourceProviderContract(item)) => {
                item.provider.key().unwrap_or("<provider>")
            }
            Some(HirItem::Instance(_)) => "instance",
            Some(HirItem::Use(_)) => "use",
            Some(HirItem::Export(_)) => "export",
            Some(HirItem::Hoist(_)) => "hoist",
            None => "<missing>",
        };
        match stage {
            Some(stage) => format!("{owner_name} stage {}", stage + 1).into_boxed_str(),
            None => format!("{owner_name} head").into_boxed_str(),
        }
    }

    fn lower_import_reference(
        &mut self,
        owner: HirItemId,
        import: ImportId,
    ) -> Result<Reference, LoweringError> {
        if let Some(target) = self
            .mock_overrides
            .get(&owner)
            .and_then(|overrides| overrides.get(&import))
            .copied()
        {
            return match target {
                MockImportTarget::Item(item_id) => Ok(self
                    .item_map
                    .get(&item_id)
                    .copied()
                    .map(Reference::Item)
                    .unwrap_or(Reference::HirItem(item_id))),
                MockImportTarget::Import(import_id) => {
                    Ok(Reference::Item(self.seed_import_item(import_id)?))
                }
            };
        }
        Ok(Reference::Item(self.seed_import_item(import)?))
    }

    fn pipe_stage_specs(&self, owner: HirItemId, pipe: &GateRuntimePipeExpr) -> Vec<PipeStageSpec> {
        let debug = self.debug_items.contains(&owner);
        let mut specs = Vec::with_capacity(
            pipe.stages.len() * usize::from(debug) + pipe.stages.len() + usize::from(debug),
        );
        if debug {
            let head_ty = Type::lower(&pipe.head.ty);
            specs.push(PipeStageSpec {
                span: pipe.head.span,
                subject_memo: None,
                result_memo: None,
                input_subject: head_ty.clone(),
                result_subject: head_ty,
                kind: PipeStageKindSpec::Debug {
                    label: self.debug_label(owner, None),
                },
            });
        }
        for (stage_index, stage) in pipe.stages.iter().enumerate() {
            specs.push(PipeStageSpec {
                span: stage.span,
                subject_memo: stage.subject_memo,
                result_memo: stage.result_memo,
                input_subject: Type::lower(&stage.input_subject),
                result_subject: Type::lower(&stage.result_subject),
                kind: match &stage.kind {
                    GateRuntimePipeStageKind::Transform { mode, .. } => {
                        PipeStageKindSpec::Transform { mode: *mode }
                    }
                    GateRuntimePipeStageKind::Tap { .. } => PipeStageKindSpec::Tap,
                    GateRuntimePipeStageKind::Gate {
                        emits_negative_update,
                        ..
                    } => PipeStageKindSpec::Gate {
                        emits_negative_update: *emits_negative_update,
                    },
                    GateRuntimePipeStageKind::Case { arms } => PipeStageKindSpec::Case {
                        arms: arms
                            .iter()
                            .map(|arm| CaseArmSpec {
                                span: arm.span,
                                pattern: arm.pattern,
                                subject: stage.input_subject.clone(),
                            })
                            .collect(),
                    },
                    GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                        PipeStageKindSpec::TruthyFalsy {
                            truthy: TruthyFalsyArmSpec::from_hir(truthy),
                            falsy: TruthyFalsyArmSpec::from_hir(falsy),
                        }
                    }
                    GateRuntimePipeStageKind::FanOut { .. } => PipeStageKindSpec::FanOut,
                },
            });
            if debug {
                let result_subject = Type::lower(&stage.result_subject);
                specs.push(PipeStageSpec {
                    span: stage.span,
                    subject_memo: None,
                    result_memo: None,
                    input_subject: result_subject.clone(),
                    result_subject,
                    kind: PipeStageKindSpec::Debug {
                        label: self.debug_label(owner, Some(stage_index)),
                    },
                });
            }
        }
        specs
    }

    fn build(mut self) -> Result<Module, LoweringErrors> {
        // Build import-to-module map for the current entry HIR so that seed_import_item
        // can find pre-compiled workspace items by module name.
        self.import_to_module = Self::make_import_to_module_map(self.hir);
        self.seed_items()?;
        self.lower_general_exprs();
        // Guard: all items must have complete elaboration before any subsequent lowering
        // step is allowed to run. Continuing with missing bodies would let gate, fanout,
        // recurrence, and source passes silently process items whose body is None, producing
        // spurious downstream errors that obscure the root cause.
        // Note: BlockedGeneralExpr errors from lower_general_exprs are tolerated here —
        // they reflect type-check failures on individual expressions, not missing elaboration
        // for an item as a whole. The gate/fanout/recurrence passes handle them downstream.
        let has_completeness_errors = self.errors.iter().any(|e| {
            matches!(
                e,
                LoweringError::MissingGeneralExprElaboration { .. }
                    | LoweringError::MissingDomainMemberElaboration { .. }
                    | LoweringError::MissingInstanceMemberElaboration { .. }
            )
        });
        if has_completeness_errors {
            return Err(LoweringErrors::new(self.errors));
        }
        self.seed_signal_dependencies();
        self.lower_gate_stages();
        self.lower_truthy_falsy_stages();
        self.lower_fanout_stages();
        self.lower_temporal_stages();
        self.lower_recurrences();
        self.finalize_pipes()?;
        self.lower_sources()?;
        self.lower_decode_programs()?;

        // Eagerly seed stub Input signal items for all workspace Signal imports so that
        // cross-module signals appear in the compiled backend. This allows markup expressions
        // that reference imported signals to find them during fragment compilation and
        // ensures the runtime assembly has signal handles for all required globals.
        self.seed_all_signal_import_stubs();

        if !self.errors.is_empty() {
            return Err(LoweringErrors::new(self.errors));
        }

        if let Err(validation) = validate_module(&self.module) {
            self.errors.extend(
                validation
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::Validation),
            );
            return Err(LoweringErrors::new(self.errors));
        }

        Ok(self.module)
    }

    fn seed_items(&mut self) -> Result<(), LoweringErrors> {
        for (hir_id, item) in self.hir.items().iter() {
            if !self.includes_item(hir_id) {
                continue;
            }
            let (span, name, kind) = match item {
                HirItem::Value(item) => {
                    (item.header.span, item.name.text().into(), ItemKind::Value)
                }
                HirItem::Function(item) => (
                    item.header.span,
                    item.name.text().into(),
                    ItemKind::Function,
                ),
                HirItem::Signal(item) => (
                    item.header.span,
                    item.name.text().into(),
                    ItemKind::Signal(SignalInfo::default()),
                ),
                HirItem::Instance(item) => (
                    item.header.span,
                    format!("instance#{}", hir_id.as_raw()).into_boxed_str(),
                    ItemKind::Instance,
                ),
                HirItem::Type(_)
                | HirItem::Class(_)
                | HirItem::Domain(_)
                | HirItem::SourceProviderContract(_)
                | HirItem::Use(_)
                | HirItem::Export(_)
                | HirItem::Hoist(_) => continue,
            };
            let item_id = self
                .module
                .items_mut()
                .alloc(Item {
                    // Add item_origin_offset to make workspace module origins non-overlapping
                    // with entry module origins (which use the raw 0-based HIR item IDs).
                    origin: HirItemId::from_raw(
                        hir_id.as_raw().saturating_add(self.item_origin_offset),
                    ),
                    span,
                    name,
                    kind,
                    parameters: Vec::new(),
                    body: None,
                    pipes: Vec::new(),
                })
                .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("items", overflow)]))?;
            self.item_map.insert(hir_id, item_id);
        }
        for (hir_id, item) in self.hir.items().iter() {
            if !self.includes_item(hir_id) {
                continue;
            }
            match item {
                HirItem::Domain(domain) => {
                    for (member_index, member) in domain.members.iter().enumerate() {
                        if member.body.is_none() {
                            continue;
                        }
                        if self.seed_domain_member_item(hir_id, member_index).is_none() {
                            break;
                        }
                    }
                }
                HirItem::Instance(instance) => {
                    for member_index in 0..instance.members.len() {
                        if self
                            .seed_instance_member_item(hir_id, member_index)
                            .is_none()
                        {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn lower_general_exprs(&mut self) {
        let report = elaborate_general_expressions(self.hir);
        self.errors
            .extend(validate_general_expr_report_completeness(
                self.hir,
                &report,
                |item_id| self.includes_item(item_id),
            ));
        let (items, domain_members, instance_members) = report.into_parts();
        for item in items {
            if !self.includes_item(item.owner) {
                continue;
            }
            let Some(owner) = self.item_map.get(&item.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: item.owner });
                continue;
            };
            self.lower_general_expr_body(
                item.owner,
                owner,
                item.body_expr,
                item.parameters,
                item.outcome,
            );
        }
        for member in domain_members {
            if !self.includes_item(member.domain_owner) {
                continue;
            }
            let key = DomainMemberKey {
                domain: member.domain_owner,
                member_index: member.member_index,
            };
            let Some(owner) = self.domain_member_item_map.get(&key).copied() else {
                self.errors.push(LoweringError::UnknownOwner {
                    owner: member.domain_owner,
                });
                continue;
            };
            self.lower_general_expr_body(
                member.domain_owner,
                owner,
                member.body_expr,
                member.parameters,
                member.outcome,
            );
        }
        for member in instance_members {
            if !self.includes_item(member.instance_owner) {
                continue;
            }
            let key = InstanceMemberKey {
                instance: member.instance_owner,
                member_index: member.member_index,
            };
            let Some(owner) = self.instance_member_item_map.get(&key).copied() else {
                self.errors.push(LoweringError::UnknownOwner {
                    owner: member.instance_owner,
                });
                continue;
            };
            self.lower_general_expr_body(
                member.instance_owner,
                owner,
                member.body_expr,
                member.parameters,
                member.outcome,
            );
        }
        // Elaborate ambient prelude items separately.  These are polymorphic helpers whose open
        // types are acceptable at the runtime level (type parameters are erased to Domain layouts
        // by the backend).  Errors from blocked ambient items are suppressed — see
        // `lower_general_expr_body`.
        let ambient_report = elaborate_ambient_items(self.hir);
        let (ambient_items, ambient_domain_members, _) = ambient_report.into_parts();
        for item in ambient_items {
            if !self.includes_item(item.owner) {
                continue;
            }
            let Some(owner) = self.item_map.get(&item.owner).copied() else {
                continue;
            };
            self.lower_general_expr_body(
                item.owner,
                owner,
                item.body_expr,
                item.parameters,
                item.outcome,
            );
        }
        for member in ambient_domain_members {
            if !self.includes_item(member.domain_owner) {
                continue;
            }
            let key = DomainMemberKey {
                domain: member.domain_owner,
                member_index: member.member_index,
            };
            let Some(owner) = self.domain_member_item_map.get(&key).copied() else {
                continue;
            };
            self.lower_general_expr_body(
                member.domain_owner,
                owner,
                member.body_expr,
                member.parameters,
                member.outcome,
            );
        }
    }

    fn lower_general_expr_body(
        &mut self,
        hir_owner: HirItemId,
        core_owner: ItemId,
        body_expr: HirExprId,
        parameters: Vec<GeneralExprParameter>,
        outcome: GeneralExprOutcome,
    ) {
        match outcome {
            GeneralExprOutcome::Lowered(body) => {
                let body = match self.lower_runtime_expr(hir_owner, &body) {
                    Ok(body) => body,
                    Err(error) => {
                        self.errors.push(error);
                        return;
                    }
                };
                let parameters = parameters
                    .into_iter()
                    .map(|parameter| ItemParameter {
                        binding: parameter.binding,
                        span: parameter.span,
                        name: parameter.name,
                        ty: Type::lower(&parameter.ty),
                    })
                    .collect::<Vec<_>>();
                let Some(core_item) = self.module.items_mut().get_mut(core_owner) else {
                    self.errors
                        .push(LoweringError::UnknownOwner { owner: hir_owner });
                    return;
                };
                core_item.parameters = parameters;
                core_item.body = Some(body);
            }
            GeneralExprOutcome::Blocked(blocked) => {
                // Ambient prelude items that fail elaboration (e.g. due to open types in
                // accumulators) are silently skipped: they'll have body=None which is acceptable
                // because any call-site that reaches them will get a runtime link error rather
                // than blocking the entire compilation.
                if self.hir.ambient_items().contains(&hir_owner) {
                    return;
                }
                if !blocked.requires_typed_core_error() {
                    return;
                }
                let span = blocked
                    .primary_span()
                    .unwrap_or(self.hir.exprs()[body_expr].span);
                self.errors.push(LoweringError::BlockedGeneralExpr {
                    owner: hir_owner,
                    body_expr,
                    span,
                    blocked,
                });
            }
        }
    }

    fn seed_signal_dependencies(&mut self) {
        for (hir_id, item) in self.hir.items().iter() {
            if !self.includes_item(hir_id) {
                continue;
            }
            let HirItem::Signal(signal) = item else {
                continue;
            };
            let Some(item_id) = self.item_map.get(&hir_id).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: hir_id });
                continue;
            };
            let mut dependencies = signal
                .signal_dependencies
                .iter()
                .filter_map(|dependency| self.map_dependency(hir_id, *dependency))
                .collect::<Vec<_>>();
            // Also include imported workspace signal dependencies.
            // These are tracked in import_item_map by ImportId, not in item_map.
            for &import_id in &signal.import_signal_dependencies {
                match self.import_item_map.get(&import_id).copied() {
                    Some(core_id) => dependencies.push(core_id),
                    None => self.errors.push(LoweringError::DependencyOutsideCore {
                        owner: hir_id,
                        dependency: HirItemId::from_raw(self.hir_item_count + import_id.as_raw()),
                    }),
                }
            }
            let Some(item) = self.module.items_mut().get_mut(item_id) else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: hir_id });
                continue;
            };
            let ItemKind::Signal(info) = &mut item.kind else {
                continue;
            };
            dependencies.sort();
            dependencies.dedup();
            info.dependencies = dependencies;
        }
    }

    fn lower_gate_stages(&mut self) {
        for stage in elaborate_gates(self.hir).into_stages() {
            if !self.item_map.contains_key(&stage.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: stage.owner });
                continue;
            }
            let key = PipeKey {
                owner: stage.owner,
                pipe_expr: stage.pipe_expr,
            };
            let lowered = match stage.outcome {
                GateStageOutcome::Ordinary(plan) => {
                    let input_subject = Type::lower(&plan.input_subject);
                    let result_subject = Type::lower(&plan.result_type);
                    let ambient = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: input_subject.clone(),
                            kind: ExprKind::AmbientSubject,
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let when_true = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: result_subject.clone(),
                            kind: ExprKind::OptionSome { payload: ambient },
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let when_false = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: result_subject.clone(),
                            kind: ExprKind::OptionNone,
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject,
                        result_subject,
                        kind: StageKind::Gate(GateStage::Ordinary {
                            when_true,
                            when_false,
                        }),
                    }
                }
                GateStageOutcome::SignalFilter(plan) => {
                    let predicate =
                        match self.lower_runtime_expr(stage.owner, &plan.runtime_predicate) {
                            Ok(expr) => expr,
                            Err(error) => {
                                self.errors.push(error);
                                continue;
                            }
                        };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::Gate(GateStage::SignalFilter {
                            payload_type: Type::lower(&plan.payload_type),
                            predicate,
                            emits_negative_update: plan.emits_negative_update,
                        }),
                    }
                }
                GateStageOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedGateStage {
                        owner: stage.owner,
                        pipe_expr: stage.pipe_expr,
                        stage_index: stage.stage_index,
                        span: stage.stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder.stages.insert(stage.stage_index, lowered).is_some() {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: stage.owner,
                    pipe_expr: stage.pipe_expr,
                    stage_index: stage.stage_index,
                });
            }
        }
    }

    fn lower_truthy_falsy_stages(&mut self) {
        for stage in elaborate_truthy_falsy(self.hir).into_stages() {
            if !self.item_map.contains_key(&stage.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: stage.owner });
                continue;
            }
            let key = PipeKey {
                owner: stage.owner,
                pipe_expr: stage.pipe_expr,
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            let outcome = match stage.outcome {
                TruthyFalsyStageOutcome::Planned(plan) => {
                    let span = join_spans(stage.truthy_stage_span, stage.falsy_stage_span);
                    PendingStage::Lowered {
                        span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::TruthyFalsy(TruthyFalsyStage {
                            truthy_stage_index: stage.truthy_stage_index,
                            truthy_stage_span: stage.truthy_stage_span,
                            falsy_stage_index: stage.falsy_stage_index,
                            falsy_stage_span: stage.falsy_stage_span,
                            truthy: TruthyFalsyBranch {
                                constructor: plan.truthy.constructor,
                                payload_subject: plan
                                    .truthy
                                    .payload_subject
                                    .as_ref()
                                    .map(Type::lower),
                                result_type: Type::lower(&plan.truthy.result_type),
                                origin_expr: plan.truthy.expr,
                            },
                            falsy: TruthyFalsyBranch {
                                constructor: plan.falsy.constructor,
                                payload_subject: plan
                                    .falsy
                                    .payload_subject
                                    .as_ref()
                                    .map(Type::lower),
                                result_type: Type::lower(&plan.falsy.result_type),
                                origin_expr: plan.falsy.expr,
                            },
                        }),
                    }
                }
                TruthyFalsyStageOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedTruthyFalsyStage {
                        owner: stage.owner,
                        pipe_expr: stage.pipe_expr,
                        truthy_stage_index: stage.truthy_stage_index,
                        falsy_stage_index: stage.falsy_stage_index,
                        span: join_spans(stage.truthy_stage_span, stage.falsy_stage_span),
                        blocked,
                    });
                    continue;
                }
            };
            if builder
                .stages
                .insert(stage.truthy_stage_index, outcome)
                .is_some()
            {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: stage.owner,
                    pipe_expr: stage.pipe_expr,
                    stage_index: stage.truthy_stage_index,
                });
            }
        }
    }

    fn lower_fanout_stages(&mut self) {
        for segment in elaborate_fanouts(self.hir).into_segments() {
            if !self.item_map.contains_key(&segment.owner) {
                self.errors.push(LoweringError::UnknownOwner {
                    owner: segment.owner,
                });
                continue;
            }
            let key = PipeKey {
                owner: segment.owner,
                pipe_expr: segment.pipe_expr,
            };
            let outcome = match segment.outcome {
                aivi_hir::FanoutSegmentOutcome::Planned(plan) => {
                    let span = plan
                        .join
                        .as_ref()
                        .map(|join| join_spans(segment.map_stage_span, join.stage_span))
                        .unwrap_or(segment.map_stage_span);
                    let mut filters = Vec::with_capacity(plan.filters.len());
                    let mut failed = false;
                    for filter in &plan.filters {
                        match self.lower_fanout_filter(segment.owner, filter) {
                            Ok(filter) => filters.push(filter),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    let runtime_map =
                        match self.lower_runtime_expr(segment.owner, &plan.runtime_map) {
                            Ok(runtime_map) => runtime_map,
                            Err(error) => {
                                self.errors.push(error);
                                continue;
                            }
                        };
                    let join = if let Some(join) = plan.join {
                        let runtime_expr =
                            match self.lower_runtime_expr(segment.owner, &join.runtime_expr) {
                                Ok(runtime_expr) => runtime_expr,
                                Err(error) => {
                                    self.errors.push(error);
                                    continue;
                                }
                            };
                        Some(FanoutJoin {
                            stage_index: join.stage_index,
                            stage_span: join.stage_span,
                            origin_expr: join.expr,
                            input_subject: Type::lower(&join.input_subject),
                            collection_subject: Type::lower(&join.collection_subject),
                            runtime_expr,
                            result_type: Type::lower(&join.result_type),
                        })
                    } else {
                        None
                    };
                    PendingStage::Lowered {
                        span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::Fanout(FanoutStage {
                            carrier: plan.carrier,
                            element_subject: Type::lower(&plan.element_subject),
                            mapped_element_type: Type::lower(&plan.mapped_element_type),
                            mapped_collection_type: Type::lower(&plan.mapped_collection_type),
                            runtime_map,
                            filters,
                            join,
                        }),
                    }
                }
                aivi_hir::FanoutSegmentOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedFanoutStage {
                        owner: segment.owner,
                        pipe_expr: segment.pipe_expr,
                        map_stage_index: segment.map_stage_index,
                        span: segment.map_stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder
                .stages
                .insert(segment.map_stage_index, outcome)
                .is_some()
            {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: segment.owner,
                    pipe_expr: segment.pipe_expr,
                    stage_index: segment.map_stage_index,
                });
            }
        }
    }

    fn lower_temporal_stages(&mut self) {
        for stage in elaborate_temporal_stages(self.hir).into_stages() {
            if !self.item_map.contains_key(&stage.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: stage.owner });
                continue;
            }
            let key = PipeKey {
                owner: stage.owner,
                pipe_expr: stage.pipe_expr,
            };
            let lowered = match stage.outcome {
                TemporalStageOutcome::Previous(plan) => {
                    let seed_expr = match self.lower_runtime_expr(stage.owner, &plan.seed_expr) {
                        Ok(expr) => expr,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_subject),
                        kind: StageKind::Temporal(TemporalStage::Previous { seed_expr }),
                    }
                }
                TemporalStageOutcome::Diff(plan) => {
                    let kind = match plan.mode {
                        aivi_hir::DiffStageMode::Function { diff_expr } => {
                            let diff_expr = match self.lower_runtime_expr(stage.owner, &diff_expr) {
                                Ok(expr) => expr,
                                Err(error) => {
                                    self.errors.push(error);
                                    continue;
                                }
                            };
                            TemporalStage::DiffFunction { diff_expr }
                        }
                        aivi_hir::DiffStageMode::Seed { seed_expr } => {
                            let seed_expr = match self.lower_runtime_expr(stage.owner, &seed_expr) {
                                Ok(expr) => expr,
                                Err(error) => {
                                    self.errors.push(error);
                                    continue;
                                }
                            };
                            TemporalStage::DiffSeed { seed_expr }
                        }
                    };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_subject),
                        kind: StageKind::Temporal(kind),
                    }
                }
                TemporalStageOutcome::Delay(plan) => {
                    let duration_expr =
                        match self.lower_runtime_expr(stage.owner, &plan.duration_expr) {
                            Ok(expr) => expr,
                            Err(error) => {
                                self.errors.push(error);
                                continue;
                            }
                        };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_subject),
                        kind: StageKind::Temporal(TemporalStage::Delay { duration_expr }),
                    }
                }
                TemporalStageOutcome::Burst(plan) => {
                    let every_expr = match self.lower_runtime_expr(stage.owner, &plan.every_expr) {
                        Ok(expr) => expr,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let count_expr = match self.lower_runtime_expr(stage.owner, &plan.count_expr) {
                        Ok(expr) => expr,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_subject),
                        kind: StageKind::Temporal(TemporalStage::Burst {
                            every_expr,
                            count_expr,
                        }),
                    }
                }
                TemporalStageOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedTemporalStage {
                        owner: stage.owner,
                        pipe_expr: stage.pipe_expr,
                        stage_index: stage.stage_index,
                        span: stage.stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder.stages.insert(stage.stage_index, lowered).is_some() {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: stage.owner,
                    pipe_expr: stage.pipe_expr,
                    stage_index: stage.stage_index,
                });
            }
        }
    }

    fn lower_recurrences(&mut self) {
        for node in elaborate_recurrences(self.hir).into_nodes() {
            if !self.item_map.contains_key(&node.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            }
            let key = PipeKey {
                owner: node.owner,
                pipe_expr: node.pipe_expr,
            };
            let recurrence = match node.outcome {
                RecurrenceNodeOutcome::Planned(plan) => {
                    let start = match self.lower_recurrence_stage(node.owner, &plan.start) {
                        Ok(stage) => stage,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let mut guards = Vec::with_capacity(plan.guards.len());
                    let mut failed = false;
                    for guard in &plan.guards {
                        match self.lower_recurrence_guard(node.owner, guard) {
                            Ok(guard) => guards.push(guard),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    let mut steps = Vec::with_capacity(plan.steps.len());
                    failed = false;
                    for step in &plan.steps {
                        match self.lower_recurrence_stage(node.owner, step) {
                            Ok(stage) => steps.push(stage),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    let non_source_wakeup = match plan.non_source_wakeup {
                        Some(binding) => {
                            match self.lower_runtime_expr(node.owner, &binding.runtime_witness) {
                                Ok(runtime_witness) => Some(NonSourceWakeup {
                                    cause: binding.cause,
                                    witness_expr: binding.witness,
                                    runtime_witness,
                                }),
                                Err(error) => {
                                    self.errors.push(error);
                                    continue;
                                }
                            }
                        }
                        None => None,
                    };
                    let seed_expr = match self.lower_runtime_expr(node.owner, &plan.seed) {
                        Ok(expr) => expr,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    PipeRecurrence {
                        target: plan.target,
                        wakeup: plan.wakeup,
                        seed_expr,
                        start,
                        guards,
                        steps,
                        non_source_wakeup,
                    }
                }
                RecurrenceNodeOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedRecurrence {
                        owner: node.owner,
                        pipe_expr: node.pipe_expr,
                        start_stage_index: node.start_stage_index,
                        span: node.start_stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder.recurrence.replace(recurrence).is_some() {
                self.errors.push(LoweringError::DuplicatePipeRecurrence {
                    owner: node.owner,
                    pipe_expr: node.pipe_expr,
                });
            }
            if let Some(&owner) = self.item_map.get(&node.owner)
                && let Some(item) = self.module.items_mut().get_mut(owner) {
                    item.body = None;
                }
        }
    }

    fn finalize_pipes(&mut self) -> Result<(), LoweringErrors> {
        let builders = std::mem::take(&mut self.pipe_builders);
        for (_, builder) in builders {
            let pipe_id = self
                .module
                .pipes_mut()
                .alloc(Pipe {
                    owner: builder.owner,
                    origin: builder.origin,
                    stages: Vec::new(),
                    recurrence: builder.recurrence,
                })
                .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("pipes", overflow)]))?;
            let mut stage_ids = Vec::with_capacity(builder.stages.len());
            for (index, pending) in builder.stages {
                let PendingStage::Lowered {
                    span,
                    input_subject,
                    result_subject,
                    kind,
                } = pending;
                let stage_id = self
                    .module
                    .stages_mut()
                    .alloc(Stage {
                        pipe: pipe_id,
                        index,
                        span,
                        input_subject,
                        result_subject,
                        kind,
                    })
                    .map_err(|overflow| {
                        LoweringErrors::new(vec![arena_overflow("stages", overflow)])
                    })?;
                stage_ids.push(stage_id);
            }
            match self.module.pipes_mut().get_mut(pipe_id) {
                Some(pipe) => pipe.stages = stage_ids,
                None => {
                    self.errors.push(LoweringError::InternalInvariantViolated {
                        message: "pipe arena did not retain the ID returned by alloc",
                    });
                    continue;
                }
            }
            match self.module.items_mut().get_mut(builder.owner) {
                Some(item) => item.pipes.push(pipe_id),
                None => {
                    self.errors.push(LoweringError::InternalInvariantViolated {
                        message: "pipe owner item was not found in the item arena after seeding",
                    });
                    continue;
                }
            }
        }
        Ok(())
    }

    fn lower_sources(&mut self) -> Result<(), LoweringErrors> {
        for node in elaborate_source_lifecycles(self.hir).into_nodes() {
            let Some(owner) = self.item_map.get(&node.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let plan = match node.outcome {
                SourceLifecycleNodeOutcome::Planned(plan) => plan,
                SourceLifecycleNodeOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedSourceLifecycle {
                        owner: node.owner,
                        span: node.source_span,
                        blocked,
                    });
                    continue;
                }
            };
            if self.source_by_owner.contains_key(&owner) {
                self.errors
                    .push(LoweringError::DuplicateSourceOwner { owner: node.owner });
                continue;
            }
            let reconfiguration_dependencies = plan
                .reconfiguration_dependencies
                .iter()
                .filter_map(|dependency| self.map_dependency(node.owner, *dependency))
                .collect::<Vec<_>>();
            let mut arguments = Vec::with_capacity(plan.arguments.len());
            let mut failed = false;
            for argument in plan.arguments {
                match self.lower_runtime_expr(node.owner, &argument.runtime_expr) {
                    Ok(runtime_expr) => arguments.push(SourceArgumentValue {
                        origin_expr: argument.expr,
                        runtime_expr,
                    }),
                    Err(error) => {
                        self.errors.push(error);
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            let mut options = Vec::with_capacity(plan.options.len());
            for option in plan.options {
                match self.lower_runtime_expr(node.owner, &option.runtime_expr) {
                    Ok(runtime_expr) => options.push(SourceOptionValue {
                        option_span: option.option_span,
                        option_name: option.option_name.text().into(),
                        origin_expr: option.expr,
                        runtime_expr,
                    }),
                    Err(error) => {
                        self.errors.push(error);
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            let source_id = self
                .module
                .sources_mut()
                .alloc(SourceNode {
                    owner,
                    span: node.source_span,
                    instance: SourceInstanceId::from_raw(plan.instance.decorator().as_raw()),
                    provider: plan.provider,
                    teardown: plan.teardown,
                    replacement: plan.replacement,
                    arguments,
                    options,
                    reconfiguration_dependencies,
                    explicit_triggers: plan
                        .explicit_triggers
                        .into_iter()
                        .map(|binding| SourceOptionBinding {
                            option_span: binding.option_span,
                            option_name: binding.option_name.text().into(),
                            origin_expr: binding.expr,
                        })
                        .collect(),
                    active_when: plan.active_when.map(|binding| SourceOptionBinding {
                        option_span: binding.option_span,
                        option_name: binding.option_name.text().into(),
                        origin_expr: binding.expr,
                    }),
                    cancellation: plan.cancellation,
                    stale_work: plan.stale_work,
                    decode: None,
                })
                .map_err(|overflow| {
                    LoweringErrors::new(vec![arena_overflow("sources", overflow)])
                })?;
            self.source_by_owner.insert(owner, source_id);
            let Some(item) = self.module.items_mut().get_mut(owner) else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let ItemKind::Signal(info) = &mut item.kind else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            info.source = Some(source_id);
        }
        Ok(())
    }

    fn lower_decode_programs(&mut self) -> Result<(), LoweringErrors> {
        for node in generate_source_decode_programs(self.hir).into_nodes() {
            let Some(owner) = self.item_map.get(&node.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let Some(source_id) = self.source_by_owner.get(&owner).copied() else {
                match node.outcome {
                    SourceDecodeProgramOutcome::Planned(_) => {
                        self.errors
                            .push(LoweringError::MissingSourceForDecode { owner: node.owner });
                    }
                    SourceDecodeProgramOutcome::Blocked(blocked) => {
                        self.errors.push(LoweringError::BlockedDecodeProgram {
                            owner: node.owner,
                            span: node.source_span,
                            blocked,
                        });
                    }
                }
                continue;
            };
            if self.decode_by_owner.contains_key(&owner) {
                self.errors
                    .push(LoweringError::DuplicateDecodeOwner { owner: node.owner });
                continue;
            }
            let program = match node.outcome {
                SourceDecodeProgramOutcome::Planned(program) => {
                    match self.lower_decode_program(owner, &program) {
                        Ok(program) => program,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    }
                }
                SourceDecodeProgramOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedDecodeProgram {
                        owner: node.owner,
                        span: node.source_span,
                        blocked,
                    });
                    continue;
                }
            };
            let decode_id =
                self.module
                    .decode_programs_mut()
                    .alloc(program)
                    .map_err(|overflow| {
                        LoweringErrors::new(vec![arena_overflow("decode-programs", overflow)])
                    })?;
            self.decode_by_owner.insert(owner, decode_id);
            match self.module.sources_mut().get_mut(source_id) {
                Some(source) => source.decode = Some(decode_id),
                None => {
                    self.errors.push(LoweringError::InternalInvariantViolated {
                        message:
                            "source arena did not retain the ID retrieved from source_by_owner",
                    });
                    continue;
                }
            }
        }
        Ok(())
    }

    fn pipe_builder(&mut self, key: PipeKey) -> Option<&mut PipeBuilder> {
        if !self.pipe_builders.contains_key(&key) {
            let owner = self.item_map.get(&key.owner).copied();
            let Some(owner) = owner else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: key.owner });
                return None;
            };
            let span = self.hir.exprs()[key.pipe_expr].span;
            self.pipe_builders.insert(
                key,
                PipeBuilder {
                    owner,
                    origin: PipeOrigin {
                        owner: key.owner,
                        pipe_expr: key.pipe_expr,
                        span,
                    },
                    stages: BTreeMap::new(),
                    recurrence: None,
                },
            );
        }
        self.pipe_builders.get_mut(&key)
    }

    fn lower_recurrence_stage(
        &mut self,
        owner: HirItemId,
        stage: &aivi_hir::RecurrenceStagePlan,
    ) -> Result<RecurrenceStage, LoweringError> {
        Ok(RecurrenceStage {
            stage_index: stage.stage_index,
            stage_span: stage.stage_span,
            origin_expr: stage.expr,
            input_subject: Type::lower(&stage.input_subject),
            result_subject: Type::lower(&stage.result_subject),
            runtime_expr: self.lower_runtime_expr(owner, &stage.runtime_expr)?,
        })
    }

    fn lower_recurrence_guard(
        &mut self,
        owner: HirItemId,
        guard: &aivi_hir::RecurrenceGuardPlan,
    ) -> Result<RecurrenceGuard, LoweringError> {
        Ok(RecurrenceGuard {
            stage_index: guard.stage_index,
            stage_span: guard.stage_span,
            predicate_expr: guard.predicate,
            input_subject: Type::lower(&guard.input_subject),
            runtime_predicate: self.lower_runtime_expr(owner, &guard.runtime_predicate)?,
        })
    }

    fn lower_fanout_filter(
        &mut self,
        owner: HirItemId,
        filter: &aivi_hir::FanoutFilterPlan,
    ) -> Result<FanoutFilter, LoweringError> {
        Ok(FanoutFilter {
            stage_index: filter.stage_index,
            stage_span: filter.stage_span,
            predicate_expr: filter.predicate,
            input_subject: Type::lower(&filter.input_subject),
            runtime_predicate: self.lower_runtime_expr(owner, &filter.runtime_predicate)?,
        })
    }

    fn map_dependency(&mut self, owner: HirItemId, dependency: HirItemId) -> Option<ItemId> {
        match self.item_map.get(&dependency).copied() {
            Some(item) => Some(item),
            None => {
                self.errors
                    .push(LoweringError::DependencyOutsideCore { owner, dependency });
                None
            }
        }
    }

    fn lower_pattern(
        &mut self,
        pattern_id: HirPatternId,
        subject: Option<&aivi_hir::GateType>,
    ) -> Pattern {
        let pattern = self.hir.patterns()[pattern_id].clone();
        let kind = match pattern.kind {
            aivi_hir::PatternKind::Wildcard => PatternKind::Wildcard,
            aivi_hir::PatternKind::Binding(binding) => PatternKind::Binding(PatternBinding {
                binding: binding.binding,
                name: binding.name.text().into(),
            }),
            aivi_hir::PatternKind::Integer(literal) => PatternKind::Integer(literal),
            aivi_hir::PatternKind::Text(text) => PatternKind::Text(lower_text_pattern(&text)),
            aivi_hir::PatternKind::Tuple(elements) => {
                let subject_elements = match subject {
                    Some(aivi_hir::GateType::Tuple(elements)) => Some(elements.as_slice()),
                    _ => None,
                };
                PatternKind::Tuple(
                    elements
                        .iter()
                        .enumerate()
                        .map(|(index, element)| {
                            self.lower_pattern(
                                *element,
                                subject_elements.and_then(|elements| elements.get(index)),
                            )
                        })
                        .collect(),
                )
            }
            aivi_hir::PatternKind::List { elements, rest } => {
                let subject_element = match subject {
                    Some(aivi_hir::GateType::List(element)) => Some(element.as_ref()),
                    _ => None,
                };
                PatternKind::List {
                    elements: elements
                        .iter()
                        .map(|element| self.lower_pattern(*element, subject_element))
                        .collect(),
                    rest: rest.map(|rest| Box::new(self.lower_pattern(rest, subject))),
                }
            }
            aivi_hir::PatternKind::Record(fields) => {
                let subject_fields = match subject {
                    Some(aivi_hir::GateType::Record(fields)) => Some(fields.as_slice()),
                    _ => None,
                };
                PatternKind::Record(
                    fields
                        .into_iter()
                        .map(|field| {
                            let field_subject = subject_fields.and_then(|subject_fields| {
                                subject_fields
                                    .iter()
                                    .find(|candidate| candidate.name.as_str() == field.label.text())
                                    .map(|field_ty| &field_ty.ty)
                            });
                            RecordPatternField {
                                label: field.label.text().into(),
                                pattern: self.lower_pattern(field.pattern, field_subject),
                            }
                        })
                        .collect(),
                )
            }
            aivi_hir::PatternKind::Constructor { callee, arguments } => {
                let hir_field_types = subject.and_then(|subject| {
                    aivi_hir::case_pattern_field_types(self.hir, &callee, subject)
                });
                let field_types = self.pattern_field_types(&callee, subject);
                PatternKind::Constructor {
                    callee: PatternConstructor {
                        display: callee.path.to_string().into_boxed_str(),
                        reference: self.lower_term_reference(&callee),
                        field_types: field_types.clone(),
                    },
                    arguments: arguments
                        .into_iter()
                        .enumerate()
                        .map(|(index, argument)| {
                            let field_subject = hir_field_types
                                .as_ref()
                                .and_then(|field_types| field_types.get(index));
                            self.lower_pattern(argument, field_subject)
                        })
                        .collect(),
                }
            }
            aivi_hir::PatternKind::UnresolvedName(callee) => PatternKind::Constructor {
                callee: PatternConstructor {
                    display: callee.path.to_string().into_boxed_str(),
                    reference: self.lower_term_reference(&callee),
                    field_types: self.pattern_field_types(&callee, subject),
                },
                arguments: Vec::new(),
            },
        };
        Pattern {
            span: pattern.span,
            kind,
        }
    }

    fn pattern_field_types(
        &self,
        callee: &aivi_hir::TermReference,
        subject: Option<&aivi_hir::GateType>,
    ) -> Option<Vec<Type>> {
        subject
            .and_then(|subject| aivi_hir::case_pattern_field_types(self.hir, callee, subject))
            .map(|field_types| field_types.into_iter().map(|ty| Type::lower(&ty)).collect())
    }

    fn lower_term_reference(&mut self, reference: &aivi_hir::TermReference) -> Reference {
        match reference.resolution.as_ref() {
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Local(binding)) => {
                Reference::Local(*binding)
            }
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Item(item)) => self
                .hir
                .sum_constructor_handle(*item, reference.path.segments().last().text())
                .map(|mut handle| {
                    handle.item = HirItemId::from_raw(
                        handle.item.as_raw().saturating_add(self.item_origin_offset),
                    );
                    Reference::SumConstructor(handle)
                })
                .or_else(|| self.item_map.get(item).copied().map(Reference::Item))
                .unwrap_or(Reference::HirItem(*item)),
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::DomainMember(
                resolution,
            )) => {
                let key = DomainMemberKey {
                    domain: resolution.domain,
                    member_index: resolution.member_index,
                };
                self.domain_member_item_map
                    .get(&key)
                    .copied()
                    .map(Reference::Item)
                    .or_else(|| {
                        self.hir
                            .domain_member_handle(*resolution)
                            .map(Reference::DomainMember)
                    })
                    .unwrap_or(Reference::HirItem(resolution.domain))
            }
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Builtin(term)) => {
                Reference::Builtin(*term)
            }
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::IntrinsicValue(
                value,
            )) => Reference::IntrinsicValue(*value),
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Import(import)) => {
                // An imported term used in a pattern must be a sum constructor (e.g. `LightTheme`,
                // `ChooseProviderStep`). Seed the import so regular expression lowering still has
                // an item to reference, then build a SumConstructorHandle using the globally stable
                // origin of the source sum type when available. That keeps imported constructor
                // values aligned with pattern arms inside compiled workspace functions.
                let variant_name: Box<str> = reference.path.segments().last().text().into();
                if let Ok(item_id) = self.seed_import_item(*import) {
                    let origin = self
                        .workspace_constructor_origin(*import)
                        .unwrap_or(self.module.items()[item_id].origin);
                    let binding = self.hir.imports().get(*import).cloned();
                    if let Some(binding) = binding {
                        let type_name: Box<str> = match &binding.metadata {
                            ImportBindingMetadata::Value {
                                ty: ImportValueType::Named { type_name, .. },
                            }
                            | ImportBindingMetadata::IntrinsicValue {
                                ty: ImportValueType::Named { type_name, .. },
                                ..
                            } => type_name.as_str().into(),
                            // For sum constructors with payload: `SwitchView : View -> UIEvent`.
                            // Walk the Arrow chain to its final Named result to get the parent
                            // type name (e.g. "UIEvent"), not the constructor name itself.
                            ImportBindingMetadata::Value {
                                ty: ImportValueType::Arrow { .. },
                            }
                            | ImportBindingMetadata::IntrinsicValue {
                                ty: ImportValueType::Arrow { .. },
                                ..
                            } => {
                                fn arrow_result_type_name(ty: &ImportValueType) -> Option<&str> {
                                    match ty {
                                        ImportValueType::Named { type_name, .. } => {
                                            Some(type_name.as_str())
                                        }
                                        ImportValueType::Arrow { result, .. } => {
                                            arrow_result_type_name(result)
                                        }
                                        _ => None,
                                    }
                                }
                                let ty = match &binding.metadata {
                                    ImportBindingMetadata::Value { ty }
                                    | ImportBindingMetadata::IntrinsicValue { ty, .. } => ty,
                                    _ => unreachable!(),
                                };
                                arrow_result_type_name(ty)
                                    .map(Box::from)
                                    .unwrap_or_else(|| variant_name.clone())
                            }
                            _ => variant_name.clone(),
                        };
                        let field_count = match &binding.metadata {
                            ImportBindingMetadata::Value { ty }
                            | ImportBindingMetadata::IntrinsicValue { ty, .. } => {
                                fn count_arrow_params(ty: &ImportValueType) -> usize {
                                    match ty {
                                        ImportValueType::Arrow { result, .. } => {
                                            1 + count_arrow_params(result)
                                        }
                                        _ => 0,
                                    }
                                }
                                count_arrow_params(ty)
                            }
                            _ => 0,
                        };
                        return Reference::SumConstructor(SumConstructorHandle {
                            item: origin,
                            type_name,
                            variant_name,
                            field_count,
                        });
                    }
                }
                Reference::HirItem(HirItemId::from_raw(0))
            }
            aivi_hir::ResolutionState::Resolved(
                aivi_hir::TermResolution::AmbiguousDomainMembers(_),
            )
            | aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::ClassMember(_))
            | aivi_hir::ResolutionState::Resolved(
                aivi_hir::TermResolution::AmbiguousClassMembers(_),
            )
            | aivi_hir::ResolutionState::Resolved(
                aivi_hir::TermResolution::AmbiguousHoistedImports(_),
            )
            | aivi_hir::ResolutionState::Unresolved => unreachable!(
                "typed-core general-expression lowering should only see resolved constructor references"
            ),
        }
    }

    fn lower_class_member_reference(
        &mut self,
        owner: HirItemId,
        span: SourceSpan,
        dispatch: &ResolvedClassMemberDispatch,
        expr_ty: &aivi_hir::GateType,
    ) -> Result<Reference, LoweringError> {
        let (class_name, member_name) = self.class_member_names(dispatch.member);
        let subject_label = self.type_binding_label(&dispatch.subject).into_boxed_str();
        let unsupported = |reason| LoweringError::UnsupportedClassMemberDispatch {
            owner,
            span,
            class_name: class_name.clone(),
            member_name: member_name.clone(),
            subject: subject_label.clone(),
            reason,
        };
        match dispatch.implementation {
            aivi_hir::ClassMemberImplementation::SameModuleInstance {
                instance,
                member_index,
            } => {
                let key = InstanceMemberKey {
                    instance,
                    member_index,
                };
                let lowered = self
                    .instance_member_item_map
                    .get(&key)
                    .copied()
                    .ok_or_else(|| {
                        unsupported(
                            "same-module instance member body was not seeded into typed-core lowering",
                        )
                    })?;
                return Ok(Reference::Item(lowered));
            }
            aivi_hir::ClassMemberImplementation::ImportedInstance { import } => {
                return self.lower_import_reference(owner, import);
            }
            aivi_hir::ClassMemberImplementation::Builtin => {}
        }

        let intrinsic = match (class_name.as_ref(), member_name.as_ref(), &dispatch.subject) {
            ("Eq", "(==)", _) | ("Setoid", "equals", _) => {
                BuiltinClassMemberIntrinsic::StructuralEq
            }
            (
                "Semigroup",
                "append",
                TypeBinding::Type(aivi_hir::GateType::Primitive(aivi_hir::BuiltinType::Text)),
            ) => BuiltinClassMemberIntrinsic::Append(BuiltinAppendCarrier::Text),
            ("Semigroup", "append", TypeBinding::Type(aivi_hir::GateType::List(_))) => {
                BuiltinClassMemberIntrinsic::Append(BuiltinAppendCarrier::List)
            }
            (
                "Monoid",
                "empty",
                TypeBinding::Type(aivi_hir::GateType::Primitive(aivi_hir::BuiltinType::Text)),
            ) => BuiltinClassMemberIntrinsic::Empty(BuiltinAppendCarrier::Text),
            ("Monoid", "empty", TypeBinding::Type(aivi_hir::GateType::List(_))) => {
                BuiltinClassMemberIntrinsic::Empty(BuiltinAppendCarrier::List)
            }
            ("Functor", "map", TypeBinding::Constructor(binding)) => match binding.head() {
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::List)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Option)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Result)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Validation)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Signal) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Signal)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Task) => {
                    BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Task)
                }
                _ => {
                    return Err(unsupported(
                        "runtime lowering only supports map for List, Option, Result, Validation, Signal, and Task",
                    ));
                }
            },
            ("Bifunctor", "bimap", TypeBinding::Constructor(binding)) => {
                let Some(carrier) = self.builtin_bifunctor_carrier(binding.head()) else {
                    return Err(unsupported(
                        "runtime lowering only supports bimap for Result and Validation",
                    ));
                };
                BuiltinClassMemberIntrinsic::Bimap(carrier)
            }
            ("Applicative", "pure", TypeBinding::Constructor(binding)) => match binding.head() {
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::List)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Option)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Result)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Validation)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Signal) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Signal)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Task) => {
                    BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Task)
                }
                _ => {
                    return Err(unsupported(
                        "runtime lowering only supports pure for List, Option, Result, Validation, Signal, and Task",
                    ));
                }
            },
            ("Apply", "apply", TypeBinding::Constructor(binding)) => match binding.head() {
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::List)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Option)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Result)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Validation)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Signal) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Signal)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Task) => {
                    BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Task)
                }
                _ => {
                    return Err(unsupported(
                        "runtime lowering only supports apply for List, Option, Result, Validation, Signal, and Task",
                    ));
                }
            },
            ("Chain", "chain", TypeBinding::Constructor(binding)) => {
                let Some(carrier) = self.builtin_monad_carrier(binding.head()) else {
                    return Err(unsupported(
                        "runtime lowering only supports chain for List, Option, Result, and Task",
                    ));
                };
                BuiltinClassMemberIntrinsic::Chain(carrier)
            }
            ("Monad", "join", TypeBinding::Constructor(binding)) => {
                let Some(carrier) = self.builtin_monad_carrier(binding.head()) else {
                    return Err(unsupported(
                        "runtime lowering only supports join for List, Option, Result, and Task",
                    ));
                };
                BuiltinClassMemberIntrinsic::Join(carrier)
            }
            ("Foldable", "reduce", TypeBinding::Constructor(binding)) => match binding.head() {
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                    BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::List)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                    BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::Option)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                    BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::Result)
                }
                TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                    BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::Validation)
                }
                _ => {
                    return Err(unsupported(
                        "runtime lowering only supports reduce for List, Option, Result, and Validation",
                    ));
                }
            },
            ("Traversable", "traverse", TypeBinding::Constructor(binding)) => {
                let Some(traversable) = self.builtin_traversable_carrier(binding.head()) else {
                    return Err(unsupported(
                        "runtime lowering only supports traverse for List, Option, Result, and Validation",
                    ));
                };
                let Some(applicative) = self.builtin_applicative_carrier_from_gate_type(expr_ty)
                else {
                    return Err(unsupported(
                        "runtime lowering only supports traverse results in List, Option, Result, Validation, and Signal applicatives",
                    ));
                };
                BuiltinClassMemberIntrinsic::Traverse {
                    traversable,
                    applicative,
                }
            }
            ("Filterable", "filterMap", TypeBinding::Constructor(binding)) => {
                let Some(carrier) = self.builtin_filterable_carrier(binding.head()) else {
                    return Err(unsupported(
                        "runtime lowering only supports filterMap for List and Option",
                    ));
                };
                BuiltinClassMemberIntrinsic::FilterMap(carrier)
            }
            ("Ord", "compare", _) => {
                let ordering_item =
                    self.ordering_item_from_gate_type(expr_ty).ok_or_else(|| {
                        unsupported("runtime lowering could not recover the Ordering result type")
                    })?;
                let subject = match &dispatch.subject {
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::Int,
                    )) => BuiltinOrdSubject::Int,
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::Float,
                    )) => BuiltinOrdSubject::Float,
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::Decimal,
                    )) => BuiltinOrdSubject::Decimal,
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::BigInt,
                    )) => BuiltinOrdSubject::BigInt,
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::Bool,
                    )) => BuiltinOrdSubject::Bool,
                    TypeBinding::Type(aivi_hir::GateType::Primitive(
                        aivi_hir::BuiltinType::Text,
                    )) => BuiltinOrdSubject::Text,
                    TypeBinding::Type(aivi_hir::GateType::OpaqueItem { name, .. })
                        if name == "Ordering" =>
                    {
                        BuiltinOrdSubject::Ordering
                    }
                    _ => {
                        return Err(unsupported(
                            "runtime lowering only supports compare for Int, Float, Decimal, BigInt, Bool, Text, and Ordering",
                        ));
                    }
                };
                BuiltinClassMemberIntrinsic::Compare {
                    subject,
                    ordering_item,
                }
            }
            _ => {
                return Err(unsupported(
                    "this builtin class member is not yet wired into typed-core lowering",
                ));
            }
        };
        Ok(Reference::BuiltinClassMember(intrinsic))
    }

    fn class_member_names(
        &self,
        resolution: aivi_hir::ClassMemberResolution,
    ) -> (Box<str>, Box<str>) {
        let class_name = match &self.hir.items()[resolution.class] {
            aivi_hir::Item::Class(class_item) => class_item.name.text().to_owned(),
            _ => "<class>".to_owned(),
        };
        let member_name = match &self.hir.items()[resolution.class] {
            aivi_hir::Item::Class(class_item) => class_item
                .members
                .get(resolution.member_index)
                .map(|member| member.name.text().to_owned())
                .unwrap_or_else(|| "<member>".to_owned()),
            _ => "<member>".to_owned(),
        };
        (class_name.into_boxed_str(), member_name.into_boxed_str())
    }

    fn type_binding_label(&self, binding: &TypeBinding) -> String {
        match binding {
            TypeBinding::Type(ty) => ty.to_string(),
            TypeBinding::Constructor(binding) => {
                let head = match binding.head() {
                    TypeConstructorHead::Builtin(builtin) => format!("{builtin:?}"),
                    TypeConstructorHead::Item(item_id) => match &self.hir.items()[item_id] {
                        aivi_hir::Item::Type(item) => item.name.text().to_owned(),
                        aivi_hir::Item::Domain(item) => item.name.text().to_owned(),
                        aivi_hir::Item::Class(item) => item.name.text().to_owned(),
                        _ => "<constructor>".to_owned(),
                    },
                    TypeConstructorHead::Import(import_id) => {
                        self.hir.imports()[import_id].local_name.text().to_owned()
                    }
                };
                if binding.arguments().is_empty() {
                    head
                } else {
                    let suffix = binding
                        .arguments()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{head} {suffix}")
                }
            }
        }
    }

    fn ordering_item_from_gate_type(&self, ty: &aivi_hir::GateType) -> Option<HirItemId> {
        let mut current = ty;
        while let aivi_hir::GateType::Arrow { result, .. } = current {
            current = result.as_ref();
        }
        match current {
            aivi_hir::GateType::OpaqueItem { item, name, .. } if name == "Ordering" => Some(*item),
            _ => None,
        }
    }

    fn builtin_bifunctor_carrier(
        &self,
        head: TypeConstructorHead,
    ) -> Option<BuiltinBifunctorCarrier> {
        match head {
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                Some(BuiltinBifunctorCarrier::Result)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                Some(BuiltinBifunctorCarrier::Validation)
            }
            _ => None,
        }
    }

    fn builtin_traversable_carrier(
        &self,
        head: TypeConstructorHead,
    ) -> Option<BuiltinTraversableCarrier> {
        match head {
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                Some(BuiltinTraversableCarrier::List)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                Some(BuiltinTraversableCarrier::Option)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                Some(BuiltinTraversableCarrier::Result)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Validation) => {
                Some(BuiltinTraversableCarrier::Validation)
            }
            _ => None,
        }
    }

    fn builtin_filterable_carrier(
        &self,
        head: TypeConstructorHead,
    ) -> Option<BuiltinFilterableCarrier> {
        match head {
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                Some(BuiltinFilterableCarrier::List)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                Some(BuiltinFilterableCarrier::Option)
            }
            _ => None,
        }
    }

    fn builtin_monad_carrier(&self, head: TypeConstructorHead) -> Option<BuiltinMonadCarrier> {
        match head {
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::List) => {
                Some(BuiltinMonadCarrier::List)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Option) => {
                Some(BuiltinMonadCarrier::Option)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Result) => {
                Some(BuiltinMonadCarrier::Result)
            }
            TypeConstructorHead::Builtin(aivi_hir::BuiltinType::Task) => {
                Some(BuiltinMonadCarrier::Task)
            }
            _ => None,
        }
    }

    fn builtin_applicative_carrier_from_gate_type(
        &self,
        ty: &aivi_hir::GateType,
    ) -> Option<BuiltinApplicativeCarrier> {
        let mut current = ty;
        while let aivi_hir::GateType::Arrow { result, .. } = current {
            current = result.as_ref();
        }
        match current {
            aivi_hir::GateType::List(_) => Some(BuiltinApplicativeCarrier::List),
            aivi_hir::GateType::Option(_) => Some(BuiltinApplicativeCarrier::Option),
            aivi_hir::GateType::Result { .. } => Some(BuiltinApplicativeCarrier::Result),
            aivi_hir::GateType::Validation { .. } => Some(BuiltinApplicativeCarrier::Validation),
            aivi_hir::GateType::Signal(_) => Some(BuiltinApplicativeCarrier::Signal),
            _ => None,
        }
    }

    fn alloc_expr(
        &mut self,
        _owner: HirItemId,
        _span: SourceSpan,
        expr: Expr,
    ) -> Result<ExprId, LoweringError> {
        self.module
            .exprs_mut()
            .alloc(expr)
            .map_err(|overflow: ArenaOverflow| LoweringError::ArenaOverflow {
                arena: "exprs",
                attempted_len: overflow.attempted_len(),
            })
    }

    fn next_synthetic_item_origin(&mut self) -> Result<HirItemId, LoweringError> {
        let raw = self.next_synthetic_item_origin_raw;
        self.next_synthetic_item_origin_raw = self
            .next_synthetic_item_origin_raw
            .checked_add(1)
            .ok_or(LoweringError::ArenaOverflow {
                arena: "synthetic import item origins",
                attempted_len: usize::MAX,
            })?;
        Ok(HirItemId::from_raw(raw))
    }

    fn next_synthetic_binding(&mut self) -> Result<aivi_hir::BindingId, LoweringError> {
        let raw = self.next_synthetic_binding_raw;
        self.next_synthetic_binding_raw =
            self.next_synthetic_binding_raw
                .checked_add(1)
                .ok_or(LoweringError::ArenaOverflow {
                    arena: "synthetic import bindings",
                    attempted_len: usize::MAX,
                })?;
        Ok(aivi_hir::BindingId::from_raw(raw))
    }

    fn lower_import_type(&self, ty: &ImportValueType) -> Type {
        Type::lower_import(ty)
    }

    fn import_item_shape(
        &mut self,
        import: ImportId,
        binding: &aivi_hir::ImportBinding,
    ) -> Result<(ItemKind, Vec<ItemParameter>), LoweringError> {
        let unsupported = |reason| LoweringError::UnsupportedImportBinding {
            import,
            span: binding.span,
            name: binding.local_name.text().into(),
            reason,
        };
        let ty = match &binding.metadata {
            ImportBindingMetadata::Value { ty }
            | ImportBindingMetadata::IntrinsicValue { ty, .. }
            | ImportBindingMetadata::InstanceMember { ty, .. } => ty,
            ImportBindingMetadata::AmbientValue { .. } => {
                return Err(unsupported(
                    "ambient imports do not carry lowered value types",
                ));
            }
            ImportBindingMetadata::OpaqueValue => {
                return Err(unsupported(
                    "opaque imports do not carry executable value types",
                ));
            }
            ImportBindingMetadata::Unknown => {
                return Err(unsupported(
                    "unresolved imports cannot be lowered into typed-core",
                ));
            }
            ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::Domain { .. }
            | ImportBindingMetadata::BuiltinType(_)
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_) => {
                return Err(unsupported(
                    "non-value imports cannot be lowered as typed-core item references",
                ));
            }
        };

        let mut parameters = Vec::new();
        let mut current = ty;
        while let ImportValueType::Arrow { parameter, result } = current {
            let parameter_index = parameters.len();
            parameters.push(ItemParameter {
                binding: self.next_synthetic_binding()?,
                span: binding.span,
                name: format!("arg{parameter_index}").into_boxed_str(),
                ty: self.lower_import_type(parameter),
            });
            current = result;
        }

        let kind = match current {
            ImportValueType::Signal(_) if parameters.is_empty() => {
                ItemKind::Signal(SignalInfo::default())
            }
            _ if parameters.is_empty() => ItemKind::Value,
            _ => ItemKind::Function,
        };
        Ok((kind, parameters))
    }

    fn workspace_constructor_origin(&self, import: ImportId) -> Option<HirItemId> {
        let binding = self.hir.imports().get(import)?;
        let module_name = binding
            .source_module
            .as_deref()
            .or_else(|| self.import_to_module.get(&import).map(|name| name.as_ref()))?;
        self.workspace_constructor_origins
            .get(module_name)?
            .get(binding.imported_name.text())
            .copied()
    }

    fn seed_import_item(&mut self, import: ImportId) -> Result<ItemId, LoweringError> {
        if let Some(item) = self.import_item_map.get(&import).copied() {
            return Ok(item);
        }
        let binding = self
            .hir
            .imports()
            .get(import)
            .ok_or(LoweringError::UnknownImport { import })?
            .clone();

        // If this import resolves to a pre-compiled workspace module item, reuse it
        // directly instead of creating a bodyless stub. Signal imports are excluded:
        // they must always use the deterministic synthetic formula (hir_item_count + import_id)
        // so the origin matches what the runtime assembly builder independently derives for
        // signal lookup. Using the workspace item's origin (which has a different base) would
        // break the assembly → backend item mapping in startup.rs index_origins.
        let (kind, parameters) = self.import_item_shape(import, &binding)?;
        if !matches!(kind, ItemKind::Signal(_))
            && let Some(module_name) = binding
                .source_module
                .as_deref()
                .or_else(|| self.import_to_module.get(&import).map(|name| name.as_ref()))
                && let Some(name_map) = self.workspace_name_maps.get(module_name)
                    && let Some(&core_item_id) = name_map.get(binding.imported_name.text()) {
                        self.import_item_map.insert(import, core_item_id);
                        return Ok(core_item_id);
                    }

        // Use a deterministic synthetic origin for Signal imports so that the runtime assembly
        // builder can independently derive the same HirItemId for the same import without
        // coordination: signal import N gets hir_item_count + N.
        let origin = if matches!(kind, ItemKind::Signal(_)) {
            HirItemId::from_raw(self.hir_item_count.checked_add(import.as_raw()).ok_or(
                LoweringError::ArenaOverflow {
                    arena: "deterministic signal import origins",
                    attempted_len: usize::MAX,
                },
            )?)
        } else {
            self.next_synthetic_item_origin()?
        };
        let body = self.synthesize_import_body(import, origin, &binding, &parameters)?;
        let item_id = self
            .module
            .items_mut()
            .alloc(Item {
                origin,
                span: binding.span,
                name: binding.local_name.text().into(),
                kind,
                parameters,
                body,
                pipes: Vec::new(),
            })
            .map_err(|overflow| LoweringError::ArenaOverflow {
                arena: "items",
                attempted_len: overflow.attempted_len(),
            })?;
        self.import_item_map.insert(import, item_id);
        Ok(item_id)
    }

    /// Eagerly seeds stub Input signal items for all workspace Signal import bindings.
    /// These stubs have no body (body=None), making them Input signals that can receive
    /// published values at runtime. The deterministic synthetic origin ensures the runtime
    /// assembly builder produces matching HirItemIds for the same imports.
    fn seed_all_signal_import_stubs(&mut self) {
        let signal_import_ids: Vec<ImportId> = self
            .hir
            .imports()
            .iter()
            .filter_map(|(import_id, binding)| {
                let is_workspace_signal = matches!(
                    &binding.metadata,
                    ImportBindingMetadata::Value {
                        ty: ImportValueType::Signal(_)
                    }
                );
                is_workspace_signal.then_some(import_id)
            })
            .collect();
        for import_id in signal_import_ids {
            // Ignore errors — these are best-effort stubs. If the import fails
            // (e.g., already seeded or shape error), skip it silently.
            let _ = self.seed_import_item(import_id);
        }
    }

    fn synthesize_import_body(
        &mut self,
        import: ImportId,
        owner: HirItemId,
        binding: &aivi_hir::ImportBinding,
        parameters: &[ItemParameter],
    ) -> Result<Option<ExprId>, LoweringError> {
        // For zero-argument sum constructor imports (e.g. `ChooseProviderStep`, `LightTheme`),
        // synthesize a `SumConstructor` expression so the value can be constructed at runtime
        // without calling a separate item body. When the constructor comes from a compiled
        // workspace module, reuse the source sum type's global origin so imported function bodies
        // and imported constructor values agree on runtime constructor identity.
        if let (
            ImportBindingMetadata::Value {
                ty: ImportValueType::Named { type_name, .. },
            },
            true,
        ) = (&binding.metadata, parameters.is_empty())
        {
            let variant_name: Box<str> = binding.local_name.text().into();
            let constructor_origin = self.workspace_constructor_origin(import).unwrap_or(owner);
            let handle = SumConstructorHandle {
                item: constructor_origin,
                type_name: type_name.as_str().into(),
                variant_name,
                field_count: 0,
            };
            let ty = self.lower_import_type(match &binding.metadata {
                ImportBindingMetadata::Value { ty } => ty,
                _ => unreachable!(),
            });
            let expr = self.alloc_expr(
                owner,
                binding.span,
                Expr {
                    span: binding.span,
                    ty,
                    kind: ExprKind::Reference(Reference::SumConstructor(handle)),
                },
            )?;
            return Ok(Some(expr));
        }

        // For N-ary sum constructor imports (e.g. `SwitchView : ViewName -> UIEvent`),
        // synthesize a body: `Apply { callee: SumConstructor(handle), arguments: [arg0, ...] }`.
        // This ensures the backend compiles a kernel body so the runtime linker can find it.
        //
        // Guard: only fire this heuristic when `callable_type` is None — constructors are
        // exported without a callable_type, while regular functions always set callable_type.
        // Without this guard a regular stdlib function like
        //   `filled : Int -> Int -> (Int -> Int -> A) -> Matrix A`
        // (result type Named("Matrix")) would be misidentified as a constructor,
        // producing a fake Sum value at runtime instead of calling the real function body.
        if let ImportBindingMetadata::Value { ty } = &binding.metadata
            && !parameters.is_empty() && binding.callable_type.is_none() {
                // Peel Arrow layers to find the result type
                let mut result_ty_ref = ty;
                for _ in parameters {
                    if let ImportValueType::Arrow { result, .. } = result_ty_ref {
                        result_ty_ref = result;
                    } else {
                        break;
                    }
                }
                // If the final result is a Named type, this is an N-ary sum constructor
                if let ImportValueType::Named { type_name, .. } = result_ty_ref {
                    let variant_name: Box<str> = binding.local_name.text().into();
                    let field_count = parameters.len();
                    let constructor_origin =
                        self.workspace_constructor_origin(import).unwrap_or(owner);
                    let handle = SumConstructorHandle {
                        item: constructor_origin,
                        type_name: type_name.as_str().into(),
                        variant_name,
                        field_count,
                    };
                    // Callee: the SumConstructor itself has the full function type
                    let callee_ty = self.lower_import_type(ty);
                    let callee_id = self.alloc_expr(
                        owner,
                        binding.span,
                        Expr {
                            span: binding.span,
                            ty: callee_ty,
                            kind: ExprKind::Reference(Reference::SumConstructor(handle)),
                        },
                    )?;
                    // Arguments: one per parameter
                    let args: Result<Vec<_>, _> = parameters
                        .iter()
                        .map(|param| {
                            self.alloc_expr(
                                owner,
                                binding.span,
                                Expr {
                                    span: binding.span,
                                    ty: param.ty.clone(),
                                    kind: ExprKind::Reference(Reference::Local(param.binding)),
                                },
                            )
                        })
                        .collect();
                    let args = args?;
                    let result_ty = self.lower_import_type(result_ty_ref);
                    let apply_id = self.alloc_expr(
                        owner,
                        binding.span,
                        Expr {
                            span: binding.span,
                            ty: result_ty,
                            kind: ExprKind::Apply {
                                callee: callee_id,
                                arguments: args,
                            },
                        },
                    )?;
                    return Ok(Some(apply_id));
                }
            }

        let ImportBindingMetadata::IntrinsicValue { value, .. } = &binding.metadata else {
            return Ok(None);
        };
        let callee = self.alloc_expr(
            owner,
            binding.span,
            Expr {
                span: binding.span,
                ty: self.lower_import_type(binding.callable_type.as_ref().unwrap_or_else(|| {
                    match &binding.metadata {
                        ImportBindingMetadata::IntrinsicValue { ty, .. } => ty,
                        _ => unreachable!("non-intrinsic imports are filtered above"),
                    }
                })),
                kind: ExprKind::Reference(Reference::IntrinsicValue(*value)),
            },
        )?;
        if parameters.is_empty() {
            return Ok(Some(callee));
        }
        let mut current = callee;
        for parameter in parameters {
            let argument = self.alloc_expr(
                owner,
                parameter.span,
                Expr {
                    span: parameter.span,
                    ty: parameter.ty.clone(),
                    kind: ExprKind::Reference(Reference::Local(parameter.binding)),
                },
            )?;
            let result_ty = match &self.module.exprs()[current].ty {
                Type::Arrow { result, .. } => result.as_ref().clone(),
                _other => {
                    return Err(LoweringError::UnsupportedImportBinding {
                        import,
                        span: binding.span,
                        name: binding.local_name.text().into(),
                        reason: "intrinsic import body expected a function before applying a synthetic parameter",
                    });
                }
            };
            current = self.alloc_expr(
                owner,
                binding.span,
                Expr {
                    span: binding.span,
                    ty: result_ty,
                    kind: ExprKind::Apply {
                        callee: current,
                        arguments: vec![argument],
                    },
                },
            )?;
        }
        Ok(Some(current))
    }

    fn seed_instance_member_item(
        &mut self,
        instance: HirItemId,
        member_index: usize,
    ) -> Option<ItemId> {
        let key = InstanceMemberKey {
            instance,
            member_index,
        };
        if let Some(item) = self.instance_member_item_map.get(&key).copied() {
            return Some(item);
        }
        let HirItem::Instance(item) = self.hir.items().get(instance)? else {
            self.errors
                .push(LoweringError::UnknownOwner { owner: instance });
            return None;
        };
        let Some(member) = item.members.get(member_index) else {
            self.errors
                .push(LoweringError::UnknownOwner { owner: instance });
            return None;
        };
        let kind = if member.parameters.is_empty() {
            ItemKind::Value
        } else {
            ItemKind::Function
        };
        // Each instance member needs a unique synthetic origin so the backend linker
        // can build a 1:1 HIR-item → backend-item index. Using the instance's HIR item
        // ID would collide: the instance itself also gets that origin from seed_items,
        // and multiple members of the same instance would share it too.
        let origin = match self.next_synthetic_item_origin() {
            Ok(id) => id,
            Err(overflow) => {
                self.errors.push(overflow);
                return None;
            }
        };
        let item_id = match self.module.items_mut().alloc(Item {
            origin,
            span: member.span,
            name: format!(
                "instance#{}::member#{}::{}",
                instance.as_raw(),
                member_index,
                member.name.text()
            )
            .into_boxed_str(),
            kind,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        }) {
            Ok(item_id) => item_id,
            Err(overflow) => {
                self.errors.push(arena_overflow("items", overflow));
                return None;
            }
        };
        self.instance_member_item_map.insert(key, item_id);
        Some(item_id)
    }

    fn seed_domain_member_item(
        &mut self,
        domain: HirItemId,
        member_index: usize,
    ) -> Option<ItemId> {
        let key = DomainMemberKey {
            domain,
            member_index,
        };
        if let Some(item) = self.domain_member_item_map.get(&key).copied() {
            return Some(item);
        }
        let HirItem::Domain(item) = self.hir.items().get(domain)? else {
            self.errors
                .push(LoweringError::UnknownOwner { owner: domain });
            return None;
        };
        let Some(member) = item.members.get(member_index) else {
            self.errors
                .push(LoweringError::UnknownOwner { owner: domain });
            return None;
        };
        let kind = if member.parameters.is_empty() {
            ItemKind::Value
        } else {
            ItemKind::Function
        };
        // Each domain member needs a unique synthetic origin so the backend
        // linker can build a 1:1 HIR-item → backend-item index without
        // spurious DuplicateBackendOrigin errors (all members of the same
        // domain would otherwise share the domain's HIR ItemId).
        let origin = match self.next_synthetic_item_origin() {
            Ok(id) => id,
            Err(overflow) => {
                self.errors.push(overflow);
                return None;
            }
        };
        let item_id = match self.module.items_mut().alloc(Item {
            origin,
            span: member.span,
            name: format!(
                "domain#{}::member#{}::{}",
                domain.as_raw(),
                member_index,
                member.name.text()
            )
            .into_boxed_str(),
            kind,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        }) {
            Ok(item_id) => item_id,
            Err(overflow) => {
                self.errors.push(arena_overflow("items", overflow));
                return None;
            }
        };
        self.domain_member_item_map.insert(key, item_id);
        Some(item_id)
    }

    fn lower_runtime_expr(
        &mut self,
        owner: HirItemId,
        root: &GateRuntimeExpr,
    ) -> Result<ExprId, LoweringError> {
        enum Task<'a> {
            Visit(&'a GateRuntimeExpr),
            BuildText {
                span: SourceSpan,
                ty: Type,
                segments: Vec<SegmentSpec>,
            },
            BuildTuple {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildList {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildMap {
                span: SourceSpan,
                ty: Type,
                entries: usize,
            },
            BuildSet {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildRecord {
                span: SourceSpan,
                ty: Type,
                labels: Vec<Box<str>>,
            },
            BuildProjection {
                span: SourceSpan,
                ty: Type,
                base_is_expr: bool,
                path: Vec<Box<str>>,
            },
            BuildApply {
                span: SourceSpan,
                ty: Type,
                arguments: usize,
            },
            BuildUnary {
                span: SourceSpan,
                ty: Type,
                operator: aivi_hir::UnaryOperator,
            },
            BuildBinary {
                span: SourceSpan,
                ty: Type,
                operator: aivi_hir::BinaryOperator,
            },
            BuildPipe {
                span: SourceSpan,
                ty: Type,
                stages: Vec<PipeStageSpec>,
            },
        }

        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr) => {
                    let ty = Type::lower(&expr.ty);
                    match &expr.kind {
                        GateRuntimeExprKind::AmbientSubject => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::AmbientSubject,
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Reference(reference) => {
                            let reference = match reference {
                                GateRuntimeReference::Local(binding) => Reference::Local(*binding),
                                GateRuntimeReference::Item(item) => self
                                    .item_map
                                    .get(item)
                                    .copied()
                                    .map(Reference::Item)
                                    .unwrap_or(Reference::HirItem(*item)),
                                GateRuntimeReference::Import(import) => {
                                    self.lower_import_reference(owner, *import)?
                                }
                                GateRuntimeReference::SumConstructor(handle) => {
                                    let mut handle = handle.clone();
                                    handle.item = HirItemId::from_raw(
                                        handle
                                            .item
                                            .as_raw()
                                            .saturating_add(self.item_origin_offset),
                                    );
                                    Reference::SumConstructor(handle)
                                }
                                GateRuntimeReference::DomainMember(handle) => self
                                    .domain_member_item_map
                                    .get(&DomainMemberKey {
                                        domain: handle.domain,
                                        member_index: handle.member_index,
                                    })
                                    .copied()
                                    .map(Reference::Item)
                                    .unwrap_or_else(|| Reference::DomainMember(handle.clone())),
                                GateRuntimeReference::ClassMember(dispatch) => self
                                    .lower_class_member_reference(
                                        owner, expr.span, dispatch, &expr.ty,
                                    )?,
                                GateRuntimeReference::Builtin(term) => Reference::Builtin(*term),
                                GateRuntimeReference::IntrinsicValue(value) => {
                                    Reference::IntrinsicValue(*value)
                                }
                            };
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::Reference(reference),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Integer(integer) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::Integer(integer.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Float(float) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::Float(float.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Decimal(decimal) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::Decimal(decimal.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::BigInt(bigint) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::BigInt(bigint.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::SuffixedInteger(integer) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::SuffixedInteger(integer.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Text(text) => {
                            tasks.push(Task::BuildText {
                                span: expr.span,
                                ty,
                                segments: text_segment_specs(text),
                            });
                            for segment in text.segments.iter().rev() {
                                if let GateRuntimeTextSegment::Interpolation(interpolation) =
                                    segment
                                {
                                    tasks.push(Task::Visit(interpolation));
                                }
                            }
                        }
                        GateRuntimeExprKind::Tuple(elements) => {
                            tasks.push(Task::BuildTuple {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::List(elements) => {
                            tasks.push(Task::BuildList {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::Map(entries) => {
                            tasks.push(Task::BuildMap {
                                span: expr.span,
                                ty,
                                entries: entries.len(),
                            });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(&entry.value));
                                tasks.push(Task::Visit(&entry.key));
                            }
                        }
                        GateRuntimeExprKind::Set(elements) => {
                            tasks.push(Task::BuildSet {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::Record(fields) => {
                            tasks.push(Task::BuildRecord {
                                span: expr.span,
                                ty,
                                labels: fields
                                    .iter()
                                    .map(|field| field.label.text().into())
                                    .collect(),
                            });
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(&field.value));
                            }
                        }
                        GateRuntimeExprKind::Projection { base, path } => {
                            tasks.push(Task::BuildProjection {
                                span: expr.span,
                                ty,
                                base_is_expr: matches!(base, GateRuntimeProjectionBase::Expr(_)),
                                path: path
                                    .segments()
                                    .iter()
                                    .map(|segment| segment.text().into())
                                    .collect(),
                            });
                            if let GateRuntimeProjectionBase::Expr(base) = base {
                                tasks.push(Task::Visit(base));
                            }
                        }
                        GateRuntimeExprKind::Apply { callee, arguments } => {
                            tasks.push(Task::BuildApply {
                                span: expr.span,
                                ty,
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument));
                            }
                            tasks.push(Task::Visit(callee));
                        }
                        GateRuntimeExprKind::Unary {
                            operator,
                            expr: inner,
                        } => {
                            tasks.push(Task::BuildUnary {
                                span: expr.span,
                                ty,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(inner));
                        }
                        GateRuntimeExprKind::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            tasks.push(Task::BuildBinary {
                                span: expr.span,
                                ty,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(right));
                            tasks.push(Task::Visit(left));
                        }
                        GateRuntimeExprKind::Pipe(pipe) => {
                            tasks.push(Task::BuildPipe {
                                span: expr.span,
                                ty,
                                stages: self.pipe_stage_specs(owner, pipe),
                            });
                            for stage in pipe.stages.iter().rev() {
                                match &stage.kind {
                                    GateRuntimePipeStageKind::Transform { expr, .. }
                                    | GateRuntimePipeStageKind::Tap { expr } => {
                                        tasks.push(Task::Visit(expr));
                                    }
                                    GateRuntimePipeStageKind::Gate { predicate, .. } => {
                                        tasks.push(Task::Visit(predicate));
                                    }
                                    GateRuntimePipeStageKind::Case { arms } => {
                                        for arm in arms.iter().rev() {
                                            tasks.push(Task::Visit(&arm.body));
                                        }
                                    }
                                    GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                                        tasks.push(Task::Visit(&falsy.body));
                                        tasks.push(Task::Visit(&truthy.body));
                                    }
                                    GateRuntimePipeStageKind::FanOut { map_expr } => {
                                        tasks.push(Task::Visit(map_expr));
                                    }
                                }
                            }
                            tasks.push(Task::Visit(&pipe.head));
                        }
                    }
                }
                Task::BuildText { span, ty, segments } => {
                    let interpolation_count = segments
                        .iter()
                        .filter(|segment| matches!(segment, SegmentSpec::Interpolation { .. }))
                        .count();
                    let mut exprs = drain_tail(&mut values, interpolation_count).into_iter();
                    let segments = segments
                        .into_iter()
                        .map(|segment| match segment {
                            SegmentSpec::Fragment { raw, span } => {
                                TextSegment::Fragment { raw, span }
                            }
                            SegmentSpec::Interpolation { span } => TextSegment::Interpolation {
                                expr: exprs.next().expect("text interpolation count should match"),
                                span,
                            },
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Text(TextLiteral { segments }),
                        },
                    )?);
                }
                Task::BuildTuple { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Tuple(elements),
                        },
                    )?);
                }
                Task::BuildList { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::List(elements),
                        },
                    )?);
                }
                Task::BuildMap { span, ty, entries } => {
                    let lowered = drain_tail(&mut values, entries * 2);
                    let mut iter = lowered.into_iter();
                    let entries = (0..entries)
                        .map(|_| MapEntry {
                            key: iter.next().expect("map key should exist"),
                            value: iter.next().expect("map value should exist"),
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Map(entries),
                        },
                    )?);
                }
                Task::BuildSet { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Set(elements),
                        },
                    )?);
                }
                Task::BuildRecord { span, ty, labels } => {
                    let len = labels.len();
                    let fields = labels
                        .into_iter()
                        .zip(drain_tail(&mut values, len))
                        .map(|(label, value)| RecordExprField { label, value })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Record(fields),
                        },
                    )?);
                }
                Task::BuildProjection {
                    span,
                    ty,
                    base_is_expr,
                    path,
                } => {
                    let base = if base_is_expr {
                        ProjectionBase::Expr(values.pop().expect("projection base should exist"))
                    } else {
                        ProjectionBase::AmbientSubject
                    };
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Projection { base, path },
                        },
                    )?);
                }
                Task::BuildApply {
                    span,
                    ty,
                    arguments,
                } => {
                    let lowered = drain_tail(&mut values, arguments + 1);
                    let mut iter = lowered.into_iter();
                    let callee = iter.next().expect("apply callee should exist");
                    let arguments = iter.collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Apply { callee, arguments },
                        },
                    )?);
                }
                Task::BuildUnary { span, ty, operator } => {
                    let inner = values.pop().expect("unary child should exist");
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Unary {
                                operator,
                                expr: inner,
                            },
                        },
                    )?);
                }
                Task::BuildBinary { span, ty, operator } => {
                    let lowered = drain_tail(&mut values, 2);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Binary {
                                left: lowered[0],
                                operator,
                                right: lowered[1],
                            },
                        },
                    )?);
                }
                Task::BuildPipe { span, ty, stages } => {
                    let lowered = drain_tail(
                        &mut values,
                        1 + stages
                            .iter()
                            .map(PipeStageSpec::child_expr_count)
                            .sum::<usize>(),
                    );
                    let mut iter = lowered.into_iter();
                    let head = iter.next().expect("pipe head should exist");
                    let stages = stages
                        .into_iter()
                        .map(|stage| {
                            let children = (0..stage.child_expr_count())
                                .map(|_| iter.next().expect("pipe stage child should exist"))
                                .collect::<Vec<_>>();
                            PipeStage {
                                span: stage.span,
                                subject_memo: stage.subject_memo,
                                result_memo: stage.result_memo,
                                input_subject: stage.input_subject,
                                result_subject: stage.result_subject,
                                kind: match stage.kind {
                                    PipeStageKindSpec::Transform { mode } => {
                                        let expr = children[0];
                                        crate::expr::PipeStageKind::Transform { mode, expr }
                                    }
                                    PipeStageKindSpec::Tap => {
                                        let expr = children[0];
                                        crate::expr::PipeStageKind::Tap { expr }
                                    }
                                    PipeStageKindSpec::Debug { label } => {
                                        crate::expr::PipeStageKind::Debug { label }
                                    }
                                    PipeStageKindSpec::Gate {
                                        emits_negative_update,
                                    } => {
                                        let predicate = children[0];
                                        crate::expr::PipeStageKind::Gate {
                                            predicate,
                                            emits_negative_update,
                                        }
                                    }
                                    PipeStageKindSpec::Case { arms } => {
                                        let mut bodies = children.into_iter();
                                        crate::expr::PipeStageKind::Case {
                                            arms: arms
                                                .into_iter()
                                                .map(|arm| PipeCaseArm {
                                                    span: arm.span,
                                                    pattern: self.lower_pattern(
                                                        arm.pattern,
                                                        Some(&arm.subject),
                                                    ),
                                                    body: bodies
                                                        .next()
                                                        .expect("case arm body should exist"),
                                                })
                                                .collect(),
                                        }
                                    }
                                    PipeStageKindSpec::TruthyFalsy { truthy, falsy } => {
                                        let mut bodies = children.into_iter();
                                        crate::expr::PipeStageKind::TruthyFalsy(
                                            PipeTruthyFalsyStage {
                                                truthy: PipeTruthyFalsyBranch {
                                                    span: truthy.span,
                                                    constructor: truthy.constructor,
                                                    payload_subject: truthy
                                                        .payload_subject
                                                        .map(|payload| Type::lower(&payload)),
                                                    result_type: Type::lower(&truthy.result_type),
                                                    body: bodies
                                                        .next()
                                                        .expect("truthy body should exist"),
                                                },
                                                falsy: PipeTruthyFalsyBranch {
                                                    span: falsy.span,
                                                    constructor: falsy.constructor,
                                                    payload_subject: falsy
                                                        .payload_subject
                                                        .map(|payload| Type::lower(&payload)),
                                                    result_type: Type::lower(&falsy.result_type),
                                                    body: bodies
                                                        .next()
                                                        .expect("falsy body should exist"),
                                                },
                                            },
                                        )
                                    }
                                    PipeStageKindSpec::FanOut => {
                                        let map_expr = children[0];
                                        crate::expr::PipeStageKind::FanOut { map_expr }
                                    }
                                },
                            }
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Pipe(PipeExpr { head, stages }),
                        },
                    )?);
                }
            }
        }

        Ok(values
            .pop()
            .expect("runtime expression lowering should produce one expression"))
    }

    fn lower_decode_program(
        &mut self,
        owner: ItemId,
        program: &SourceDecodeProgram,
    ) -> Result<DecodeProgram, LoweringError> {
        let mut steps = Arena::new();
        let step_positions = program
            .steps()
            .iter()
            .enumerate()
            .map(|(index, step)| (step as *const _, index))
            .collect::<HashMap<_, _>>();

        let step_id_for = |program: &SourceDecodeProgram,
                           step_positions: &HashMap<*const aivi_hir::DecodeProgramStep, usize>,
                           step_id: aivi_hir::DecodeProgramStepId|
         -> DecodeStepId {
            let ptr = program.step(step_id) as *const _;
            let index = step_positions[&ptr];
            DecodeStepId::from_raw(index as u32)
        };

        for step in program.steps() {
            let lowered = match step {
                aivi_hir::DecodeProgramStep::Scalar { scalar } => {
                    DecodeStep::Scalar { scalar: *scalar }
                }
                aivi_hir::DecodeProgramStep::Tuple { elements } => DecodeStep::Tuple {
                    elements: elements
                        .iter()
                        .map(|step| step_id_for(program, &step_positions, *step))
                        .collect(),
                },
                aivi_hir::DecodeProgramStep::Record {
                    fields,
                    extra_fields,
                } => DecodeStep::Record {
                    fields: fields
                        .iter()
                        .map(|field| DecodeField {
                            name: field.name.as_str().into(),
                            requirement: field.requirement,
                            step: step_id_for(program, &step_positions, field.step),
                        })
                        .collect(),
                    extra_fields: *extra_fields,
                },
                aivi_hir::DecodeProgramStep::Sum { variants, strategy } => DecodeStep::Sum {
                    variants: variants
                        .iter()
                        .map(|variant| crate::DecodeVariant {
                            name: variant.name.as_str().into(),
                            payload: variant
                                .payload
                                .map(|payload| step_id_for(program, &step_positions, payload)),
                        })
                        .collect(),
                    strategy: *strategy,
                },
                aivi_hir::DecodeProgramStep::Domain { carrier, surface } => DecodeStep::Domain {
                    carrier: step_id_for(program, &step_positions, *carrier),
                    surface: DomainDecodeSurface {
                        domain_item: surface.domain_item,
                        member_index: surface.member_index,
                        member_name: surface.member_name.clone(),
                        kind: match surface.kind {
                            aivi_hir::DomainDecodeSurfaceKind::Direct => {
                                DomainDecodeSurfaceKind::Direct
                            }
                            aivi_hir::DomainDecodeSurfaceKind::FallibleResult => {
                                DomainDecodeSurfaceKind::FallibleResult
                            }
                        },
                        span: surface.span,
                    },
                },
                aivi_hir::DecodeProgramStep::List { element } => DecodeStep::List {
                    element: step_id_for(program, &step_positions, *element),
                },
                aivi_hir::DecodeProgramStep::Option { element } => DecodeStep::Option {
                    element: step_id_for(program, &step_positions, *element),
                },
                aivi_hir::DecodeProgramStep::Result { error, value } => DecodeStep::Result {
                    error: step_id_for(program, &step_positions, *error),
                    value: step_id_for(program, &step_positions, *value),
                },
                aivi_hir::DecodeProgramStep::Validation { error, value } => {
                    DecodeStep::Validation {
                        error: step_id_for(program, &step_positions, *error),
                        value: step_id_for(program, &step_positions, *value),
                    }
                }
            };
            let _ = steps
                .alloc(lowered)
                .map_err(|overflow| LoweringError::ArenaOverflow {
                    arena: "decode-steps",
                    attempted_len: overflow.attempted_len(),
                })?;
        }

        let root_index = step_positions[&(program.root_step() as *const _)] as u32;
        Ok(DecodeProgram::new(
            owner,
            program.mode,
            program.payload_annotation,
            DecodeStepId::from_raw(root_index),
            steps,
        ))
    }
}

fn arena_overflow(arena: &'static str, overflow: ArenaOverflow) -> LoweringError {
    LoweringError::ArenaOverflow {
        arena,
        attempted_len: overflow.attempted_len(),
    }
}

fn join_spans(left: SourceSpan, right: SourceSpan) -> SourceSpan {
    left.join(right)
        .expect("typed-core lowering only joins spans from the same source file")
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("requested more lowered values than available");
    values.drain(split..).collect()
}

fn text_segment_specs(text: &GateRuntimeTextLiteral) -> Vec<SegmentSpec> {
    text.segments
        .iter()
        .map(|segment| match segment {
            GateRuntimeTextSegment::Fragment(fragment) => SegmentSpec::Fragment {
                raw: fragment.raw.clone(),
                span: fragment.span,
            },
            GateRuntimeTextSegment::Interpolation(interpolation) => SegmentSpec::Interpolation {
                span: interpolation.span,
            },
        })
        .collect()
}

fn lower_text_pattern(text: &aivi_hir::TextLiteral) -> Box<str> {
    let mut raw = String::new();
    for segment in &text.segments {
        match segment {
            aivi_hir::TextSegment::Text(fragment) => raw.push_str(&fragment.raw),
            aivi_hir::TextSegment::Interpolation(_) => raw.push_str("{...}"),
        }
    }
    raw.into_boxed_str()
}

#[derive(Clone)]
enum SegmentSpec {
    Fragment { raw: Box<str>, span: SourceSpan },
    Interpolation { span: SourceSpan },
}

#[derive(Clone)]
struct PipeStageSpec {
    span: SourceSpan,
    subject_memo: Option<aivi_hir::BindingId>,
    result_memo: Option<aivi_hir::BindingId>,
    input_subject: Type,
    result_subject: Type,
    kind: PipeStageKindSpec,
}

impl PipeStageSpec {
    fn child_expr_count(&self) -> usize {
        self.kind.child_expr_count()
    }
}

#[derive(Clone)]
enum PipeStageKindSpec {
    Transform {
        mode: PipeTransformMode,
    },
    Tap,
    Debug {
        label: Box<str>,
    },
    Gate {
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<CaseArmSpec>,
    },
    TruthyFalsy {
        truthy: TruthyFalsyArmSpec,
        falsy: TruthyFalsyArmSpec,
    },
    FanOut,
}

impl PipeStageKindSpec {
    fn child_expr_count(&self) -> usize {
        match self {
            Self::Transform { .. } | Self::Tap | Self::Gate { .. } | Self::FanOut => 1,
            Self::Debug { .. } => 0,
            Self::Case { arms } => arms.len(),
            Self::TruthyFalsy { .. } => 2,
        }
    }
}

#[derive(Clone)]
struct CaseArmSpec {
    span: SourceSpan,
    pattern: HirPatternId,
    subject: aivi_hir::GateType,
}

#[derive(Clone)]
struct TruthyFalsyArmSpec {
    span: SourceSpan,
    constructor: aivi_hir::BuiltinTerm,
    payload_subject: Option<aivi_hir::GateType>,
    result_type: aivi_hir::GateType,
}

impl TruthyFalsyArmSpec {
    fn from_hir(branch: &GateRuntimeTruthyFalsyBranch) -> Self {
        Self {
            span: branch.span,
            constructor: branch.constructor,
            payload_subject: branch.payload_subject.clone(),
            result_type: branch.result_type.clone(),
        }
    }
}

