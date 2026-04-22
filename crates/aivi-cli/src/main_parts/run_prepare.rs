enum RunPreparationStageEvent {
    Started(&'static str),
    Detail(&'static str, String),
    Completed(&'static str, Duration),
}

fn prepare_run_artifact(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    prepare_run_artifact_with_query_context(sources, module, workspace_hirs, requested_view, None)
}

fn prepare_run_artifact_with_query_context(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    requested_view: Option<&str>,
    query_context: Option<BackendQueryContext<'_>>,
) -> Result<RunArtifact, String> {
    prepare_run_artifact_with_metrics_and_query_context(
        sources,
        module,
        workspace_hirs,
        requested_view,
        query_context,
    )
    .map(|prepared| prepared.artifact)
}

fn prepare_run_artifact_with_metrics_and_query_context(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    requested_view: Option<&str>,
    query_context: Option<BackendQueryContext<'_>>,
) -> Result<PreparedRunArtifact, String> {
    prepare_run_artifact_with_metrics_and_progress(
        sources,
        module,
        workspace_hirs,
        requested_view,
        query_context,
        |_| {},
    )
}

fn prepare_run_artifact_with_metrics_and_progress<F>(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    requested_view: Option<&str>,
    query_context: Option<BackendQueryContext<'_>>,
    mut on_stage_event: F,
) -> Result<PreparedRunArtifact, String>
where
    F: FnMut(RunPreparationStageEvent),
{
    let total_started = Instant::now();
    let mut metrics = RunArtifactPreparationMetrics {
        workspace_module_count: workspace_hirs.len(),
        ..RunArtifactPreparationMetrics::default()
    };
    let included_items = production_item_ids(module);
    let selected = select_run_entry(module, requested_view)?;
    let view_owner = find_value_owner(module, selected.value).ok_or_else(|| {
        format!(
            "failed to recover owning item for run view `{}`",
            selected.value.name.text()
        )
    })?;
    let gtk_preparation = match selected.kind {
        SelectedRunEntryKind::Gtk => {
            on_stage_event(RunPreparationStageEvent::Started("markup lowering"));
            let markup_lowering_started = Instant::now();
            let plan = lower_markup_expr(module, selected.value.body).map_err(|error| {
                format!(
                    "failed to lower run view `{}` into GTK markup: {error}",
                    selected.value.name.text()
                )
            })?;
            metrics.markup_lowering = markup_lowering_started.elapsed();
            on_stage_event(RunPreparationStageEvent::Completed(
                "markup lowering",
                metrics.markup_lowering,
            ));
            on_stage_event(RunPreparationStageEvent::Started("GTK bridge lowering"));
            let widget_bridge_lowering_started = Instant::now();
            let bridge = lower_widget_bridge(&plan).map_err(|error| {
                format!(
                    "failed to lower run view `{}` into a GTK bridge graph: {error}",
                    selected.value.name.text()
                )
            })?;
            metrics.widget_bridge_lowering = widget_bridge_lowering_started.elapsed();
            on_stage_event(RunPreparationStageEvent::Completed(
                "GTK bridge lowering",
                metrics.widget_bridge_lowering,
            ));
            on_stage_event(RunPreparationStageEvent::Started("run plan validation"));
            let run_plan_validation_started = Instant::now();
            validate_run_plan(sources, &bridge)?;
            metrics.run_plan_validation = run_plan_validation_started.elapsed();
            on_stage_event(RunPreparationStageEvent::Completed(
                "run plan validation",
                metrics.run_plan_validation,
            ));
            Some(bridge)
        }
        SelectedRunEntryKind::HeadlessTaskMain => None,
    };
    on_stage_event(RunPreparationStageEvent::Started("full-program lowering"));
    let runtime_backend_lowering_started = Instant::now();
    let lowered = if workspace_hirs.is_empty() {
        if let Some(query_context) = query_context {
        let unit = whole_program_backend_unit_with_items(
            query_context.db,
            query_context.entry,
            &included_items,
        )
        .map_err(|error| format!("failed to lower `aivi run` module into backend unit: {error}"))?;
        LoweredRunBackendStack {
            core: unit.core().clone(),
            backend: unit.backend_arc(),
        }
        } else {
        lower_runtime_backend_stack_with_items_fast(module, &included_items, "`aivi run`")?
        }
    } else {
        lower_runtime_backend_stack_with_workspace(
            module,
            workspace_hirs,
            &included_items,
            "`aivi run`",
        )?
    };
    metrics.runtime_backend_lowering = runtime_backend_lowering_started.elapsed();
    on_stage_event(RunPreparationStageEvent::Completed(
        "full-program lowering",
        metrics.runtime_backend_lowering,
    ));
    metrics.runtime_backend_item_count = lowered.backend.items().iter().count();
    metrics.runtime_backend_kernel_count = lowered.backend.kernels().iter().count();
    on_stage_event(RunPreparationStageEvent::Started("runtime assembly"));
    let runtime_assembly_started = Instant::now();
    let profiled_runtime_assembly = if workspace_hirs.is_empty() {
        assemble_hir_runtime_with_items_profiled_and_progress(module, &included_items, |detail| {
            on_stage_event(RunPreparationStageEvent::Detail("runtime assembly", detail));
        })
    } else {
        assemble_hir_runtime_with_items_and_workspace_profiled_and_progress(
            module,
            workspace_hirs,
            &included_items,
            |detail| {
                on_stage_event(RunPreparationStageEvent::Detail("runtime assembly", detail));
            },
        )
    }
    .map_err(|errors| {
        let mut rendered = String::from("failed to assemble runtime plans for `aivi run`:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    metrics.runtime_assembly = runtime_assembly_started.elapsed();
    on_stage_event(RunPreparationStageEvent::Completed(
        "runtime assembly",
        metrics.runtime_assembly,
    ));
    metrics.reactive_guard_fragment_count =
        profiled_runtime_assembly.stats.reactive_guard_fragments;
    metrics.reactive_body_fragment_count = profiled_runtime_assembly.stats.reactive_body_fragments;
    metrics.reactive_fragment_compilation = profiled_runtime_assembly
        .stats
        .reactive_fragment_compile_duration;
    on_stage_event(RunPreparationStageEvent::Completed(
        "reactive fragment compile",
        metrics.reactive_fragment_compilation,
    ));
    let runtime_assembly = profiled_runtime_assembly.assembly;
    let runtime_backend_by_hir = backend_items_by_hir(&lowered.core, lowered.backend.as_ref());
    let runtime_link =
        aivi_runtime::derive_backend_runtime_link_seed(&lowered.core, lowered.backend.as_ref())
            .map_err(|errors| {
                let mut rendered = String::from(
                    "failed to derive source-free runtime link seed for `aivi run`:\n",
                );
                for error in errors.errors() {
                    rendered.push_str("- ");
                    rendered.push_str(&error.to_string());
                    rendered.push('\n');
                }
                rendered
            })?;
    let kind = if let Some(bridge) = gtk_preparation {
        on_stage_event(RunPreparationStageEvent::Started("runtime expr sites"));
        let markup_site_collection_started = Instant::now();
        let sites = collect_markup_runtime_expr_sites(module, selected.value.body).map_err(|error| {
            let span_info = match &error {
                aivi_hir::MarkupRuntimeExprSiteError::UnknownExprType { span, .. } => {
                    format!(" (failing expr at {})", source_location(sources, *span))
                }
                _ => String::new(),
            };
            format!(
                "failed to collect runtime expression environments for run view at {}: {error}{span_info}",
                source_location(sources, module.exprs()[selected.value.body].span)
            )
        })?;
        metrics.markup_site_collection = markup_site_collection_started.elapsed();
        on_stage_event(RunPreparationStageEvent::Completed(
            "runtime expr sites",
            metrics.markup_site_collection,
        ));
        on_stage_event(RunPreparationStageEvent::Started("hydration fragments"));
        let hydration_fragment_compilation_started = Instant::now();
        let (hydration_inputs, hydration_metrics) = compile_run_inputs(
            sources,
            module,
            workspace_hirs,
            view_owner,
            &sites,
            &bridge,
            lowered.backend.as_ref(),
            &runtime_backend_by_hir,
            query_context,
        )?;
        metrics.hydration_fragment_compilation = hydration_fragment_compilation_started.elapsed();
        on_stage_event(RunPreparationStageEvent::Completed(
            "hydration fragments",
            metrics.hydration_fragment_compilation,
        ));
        metrics.hydration_fragment_count = hydration_metrics.compiled_fragment_count;
        let patterns = collect_run_patterns(sources, module, &bridge)?;
        on_stage_event(RunPreparationStageEvent::Started("event handler resolve"));
        let event_handler_resolution_started = Instant::now();
        let event_handlers = resolve_run_event_handlers(
            module,
            workspace_hirs,
            &sites,
            &bridge,
            &runtime_assembly,
            sources,
        )?;
        metrics.event_handler_resolution = event_handler_resolution_started.elapsed();
        on_stage_event(RunPreparationStageEvent::Completed(
            "event handler resolve",
            metrics.event_handler_resolution,
        ));
        RunArtifactKind::Gtk(RunGtkArtifact {
            patterns,
            bridge,
            hydration_inputs,
            event_handlers,
        })
    } else {
        if runtime_assembly.task_by_owner(view_owner).is_none() {
            return Err(
                "`aivi run` headless entries require `value main : Task ...`".to_owned(),
            );
        }
        RunArtifactKind::HeadlessTask {
            task_owner: view_owner,
        }
    };
    let required_signal_globals = match &kind {
        RunArtifactKind::Gtk(surface) => collect_run_required_signal_globals(&surface.hydration_inputs),
        RunArtifactKind::HeadlessTask { .. } => BTreeMap::new(),
    };
    on_stage_event(RunPreparationStageEvent::Started("stub signal defaults"));
    let stub_signal_defaults_started = Instant::now();
    let stub_signal_defaults = collect_stub_signal_defaults(module, &runtime_assembly);
    metrics.stub_signal_defaults = stub_signal_defaults_started.elapsed();
    on_stage_event(RunPreparationStageEvent::Completed(
        "stub signal defaults",
        metrics.stub_signal_defaults,
    ));
    metrics.total = total_started.elapsed();
    let mut artifact = RunArtifact {
        view_name: selected.value.name.text().into(),
        kind,
        required_signal_globals,
        sources: Some(sources.clone()),
        runtime_assembly,
        runtime_link,
        runtime_tables: None,
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload::Program(lowered.backend),
        backend_native_kernels: Arc::new(aivi_backend::NativeKernelArtifactSet::default()),
        stub_signal_defaults,
    };
    backfill_fragment_opaque_layout_variants(&mut artifact);
    Ok(PreparedRunArtifact { artifact, metrics })
}

#[derive(Clone, Copy)]
enum SelectedRunEntryKind {
    Gtk,
    HeadlessTaskMain,
}

#[derive(Clone, Copy)]
struct SelectedRunEntry<'a> {
    value: &'a ValueItem,
    kind: SelectedRunEntryKind,
}

fn select_run_entry<'a>(
    module: &'a HirModule,
    requested_view: Option<&str>,
) -> Result<SelectedRunEntry<'a>, String> {
    let mut markup_values = Vec::new();
    let mut all_values = Vec::new();
    for (item_id, item) in module.items().iter() {
        if item_is_test(module, item_id) {
            continue;
        }
        let Item::Value(value) = item else {
            continue;
        };
        all_values.push(value);
        if matches!(module.exprs()[value.body].kind, ExprKind::Markup(_)) {
            markup_values.push(value);
        }
    }

    if let Some(requested_view) = requested_view {
        let Some(value) = all_values
            .into_iter()
            .find(|value| value.name.text() == requested_view)
        else {
            let available = markup_view_names(&markup_values);
            return Err(if available.is_empty() {
                format!(
                    "run view `{requested_view}` does not exist and this module exposes no markup-valued top-level `value`s"
                )
            } else {
                format!(
                    "run view `{requested_view}` does not exist; available markup views: {}",
                    available.join(", ")
                )
            });
        };
        return classify_selected_run_entry(module, value);
    }

    if let Some(view) = markup_values
        .iter()
        .copied()
        .find(|value| value.name.text() == "view")
    {
        return Ok(SelectedRunEntry {
            value: view,
            kind: SelectedRunEntryKind::Gtk,
        });
    }

    if markup_values.is_empty()
        && let Ok(main) = select_execute_main(module) {
            return Ok(SelectedRunEntry {
                value: main,
                kind: SelectedRunEntryKind::HeadlessTaskMain,
            });
    }

    match markup_values.as_slice() {
        [single] => Ok(SelectedRunEntry {
            value: *single,
            kind: SelectedRunEntryKind::Gtk,
        }),
        [] => Err("no markup view found; define `value view = <Window ...>` or pass `--view <name>` for another markup-valued top-level `value`".to_owned()),
        many => Err(format!(
            "multiple markup views are available ({}); rename one to `view` or pass `--view <name>`",
            markup_view_names(many).join(", ")
        )),
    }
}

fn classify_selected_run_entry<'a>(
    module: &'a HirModule,
    value: &'a ValueItem,
) -> Result<SelectedRunEntry<'a>, String> {
    if matches!(module.exprs()[value.body].kind, ExprKind::Markup(_)) {
        return Ok(SelectedRunEntry {
            value,
            kind: SelectedRunEntryKind::Gtk,
        });
    }
    if value.name.text() == "main" {
        return Ok(SelectedRunEntry {
            value,
            kind: SelectedRunEntryKind::HeadlessTaskMain,
        });
    }
    Err(format!(
        "run view `{}` exists but is not markup; `aivi run` currently supports markup-valued top-level `value`s or headless `value main : Task ...` entries",
        value.name.text()
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
fn select_run_view<'a>(
    module: &'a HirModule,
    requested_view: Option<&str>,
) -> Result<&'a ValueItem, String> {
    select_run_entry(module, requested_view).map(|selected| selected.value)
}

fn markup_view_names(values: &[&ValueItem]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.name.text().to_owned())
        .collect()
}

fn find_value_owner(module: &HirModule, value: &ValueItem) -> Option<aivi_hir::ItemId> {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(candidate)
                if candidate.body == value.body && candidate.name.text() == value.name.text() =>
            {
                Some(item_id)
            }
            _ => None,
        })
}

fn validate_run_plan(sources: &SourceDatabase, bridge: &GtkBridgeGraph) -> Result<(), String> {
    let mut blockers = Vec::<RunValidationBlocker>::new();

    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Widget(widget) => {
                let Some(schema) = lookup_widget_schema(&widget.widget) else {
                    blockers.push(RunValidationBlocker {
                        span: node.span,
                        message: format!(
                            "`aivi run` does not support GTK widget `{}` yet",
                            run_widget_name(&widget.widget)
                        ),
                    });
                    continue;
                };
                for property in &widget.properties {
                    let (name, span) = match property {
                        RuntimePropertyBinding::Static(property) => {
                            (property.name.text(), property.site.span)
                        }
                        RuntimePropertyBinding::Setter(binding) => {
                            (binding.name.text(), binding.site.span)
                        }
                    };
                    if schema.property(name).is_none() {
                        blockers.push(RunValidationBlocker {
                            span,
                            message: format!(
                                "`aivi run` does not support property `{name}` on GTK widget `{}` yet",
                                schema.markup_name
                            ),
                        });
                    }
                }
                for event in &widget.event_hooks {
                    if schema.event(event.name.text()).is_none() {
                        blockers.push(RunValidationBlocker {
                            span: event.site.span,
                            message: format!(
                                "`aivi run` does not support event `{}` on GTK widget `{}` yet",
                                event.name.text(),
                                schema.markup_name
                            ),
                        });
                    }
                }
                validate_run_widget_children(
                    node.span,
                    count_unnamed_widget_children(bridge, &widget.default_children.roots),
                    schema,
                    &mut blockers,
                );
            }
            GtkBridgeNodeKind::Group(group) => validate_run_group_children(
                node.span,
                group.body.roots.len(),
                group.descriptor,
                &mut blockers,
            ),
            GtkBridgeNodeKind::Show(_)
            | GtkBridgeNodeKind::Each(_)
            | GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Match(_)
            | GtkBridgeNodeKind::Case(_)
            | GtkBridgeNodeKind::Fragment(_)
            | GtkBridgeNodeKind::With(_) => {}
        }
    }

    let mut root_widgets = collect_run_root_widgets(bridge, bridge.root());
    root_widgets.sort();
    root_widgets.dedup();
    if root_widgets.is_empty() {
        blockers.push(RunValidationBlocker {
            span: bridge.root_node().span,
            message:
                "`aivi run` requires the selected view to produce at least one reachable GTK root widget"
                    .to_owned(),
        });
    }
    for root in root_widgets {
        let Some(node) = bridge.node(root.plan) else {
            continue;
        };
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        let Some(schema) = lookup_widget_schema(&widget.widget) else {
            continue;
        };
        if !schema.is_window_root() {
            blockers.push(RunValidationBlocker {
                span: node.span,
                message: format!(
                    "`aivi run` currently requires reachable root widgets to be `Window`; found `{}`",
                    schema.markup_name
                ),
            });
        }
    }

    if blockers.is_empty() {
        return Ok(());
    }

    let mut rendered = String::from(
        "`aivi run` does not support every GTK/runtime feature yet. Unsupported features in the selected view:\n",
    );
    for blocker in blockers {
        rendered.push_str("- ");
        rendered.push_str(&source_location(sources, blocker.span));
        rendered.push_str(": ");
        rendered.push_str(&blocker.message);
        rendered.push('\n');
    }
    Err(rendered)
}

fn collect_run_patterns(
    sources: &SourceDatabase,
    module: &HirModule,
    bridge: &GtkBridgeGraph,
) -> Result<RunPatternTable, String> {
    let mut patterns = RunPatternTable::default();
    let mut visited = BTreeSet::new();
    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Match(match_node) => {
                for branch in &match_node.cases {
                    collect_run_pattern(sources, module, branch.pattern, &mut patterns, &mut visited)?;
                }
            }
            GtkBridgeNodeKind::Case(case_node) => {
                collect_run_pattern(sources, module, case_node.pattern, &mut patterns, &mut visited)?;
            }
            GtkBridgeNodeKind::Widget(_)
            | GtkBridgeNodeKind::Group(_)
            | GtkBridgeNodeKind::Show(_)
            | GtkBridgeNodeKind::Each(_)
            | GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Fragment(_)
            | GtkBridgeNodeKind::With(_) => {}
        }
    }
    Ok(patterns)
}

fn collect_run_pattern(
    sources: &SourceDatabase,
    module: &HirModule,
    pattern_id: HirPatternId,
    patterns: &mut RunPatternTable,
    visited: &mut BTreeSet<HirPatternId>,
) -> Result<(), String> {
    if !visited.insert(pattern_id) {
        return Ok(());
    }
    let pattern = &module.patterns()[pattern_id];
    let kind = match &pattern.kind {
        PatternKind::Wildcard => RunPatternKind::Wildcard,
        PatternKind::Binding(binding) => RunPatternKind::Binding {
            binding: binding.binding,
            name: binding.name.text().into(),
        },
        PatternKind::Integer(integer) => RunPatternKind::Integer {
            raw: integer.raw.clone(),
        },
        PatternKind::Text(text) => RunPatternKind::Text {
            value: text_literal_static_text(text).ok_or_else(|| {
                format!(
                    "run artifact cannot serialize non-static text pattern at {}",
                    source_location(sources, pattern.span)
                )
            })?
            .into(),
        },
        PatternKind::Tuple(elements) => {
            let children = elements.iter().copied().collect::<Vec<_>>();
            for child in &children {
                collect_run_pattern(sources, module, *child, patterns, visited)?;
            }
            RunPatternKind::Tuple(children.into_boxed_slice())
        }
        PatternKind::List { elements, rest } => {
            for child in elements {
                collect_run_pattern(sources, module, *child, patterns, visited)?;
            }
            if let Some(rest) = rest {
                collect_run_pattern(sources, module, *rest, patterns, visited)?;
            }
            RunPatternKind::List {
                elements: elements.clone().into_boxed_slice(),
                rest: *rest,
            }
        }
        PatternKind::Record(fields) => {
            let mut run_fields = Vec::with_capacity(fields.len());
            for field in fields {
                collect_run_pattern(sources, module, field.pattern, patterns, visited)?;
                run_fields.push(RunRecordPatternField {
                    label: field.label.text().into(),
                    pattern: field.pattern,
                });
            }
            RunPatternKind::Record(run_fields.into_boxed_slice())
        }
        PatternKind::Constructor { callee, arguments } => {
            for child in arguments {
                collect_run_pattern(sources, module, *child, patterns, visited)?;
            }
            serialize_run_pattern_constructor(callee, arguments)
        }
        PatternKind::UnresolvedName(reference) => {
            serialize_run_pattern_constructor(reference, &[])
        }
    };
    patterns.insert(pattern_id, RunPattern { kind });
    Ok(())
}

fn serialize_run_pattern_constructor(
    callee: &aivi_hir::TermReference,
    arguments: &[HirPatternId],
) -> RunPatternKind {
    match callee.resolution.as_ref() {
        aivi_hir::ResolutionState::Resolved(TermResolution::Builtin(term)) => {
            RunPatternKind::Constructor {
                callee: RunPatternConstructor::Builtin(*term),
                arguments: arguments.to_vec().into_boxed_slice(),
            }
        }
        aivi_hir::ResolutionState::Resolved(TermResolution::Item(item)) => {
            RunPatternKind::Constructor {
                callee: RunPatternConstructor::Item {
                    item: *item,
                    variant_name: callee
                        .path
                        .segments()
                        .last()
                        .text()
                        .to_owned()
                        .into_boxed_str(),
                },
                arguments: arguments.to_vec().into_boxed_slice(),
            }
        }
        _ => RunPatternKind::UnresolvedName,
    }
}

fn count_unnamed_widget_children(bridge: &GtkBridgeGraph, roots: &[GtkBridgeNodeRef]) -> usize {
    roots
        .iter()
        .filter(|root| {
            !matches!(
                bridge.node(root.plan).map(|node| &node.kind),
                Some(GtkBridgeNodeKind::Group(_))
            )
        })
        .count()
}

fn validate_run_widget_children(
    span: SourceSpan,
    child_count: usize,
    schema: &aivi_gtk::GtkWidgetSchema,
    blockers: &mut Vec<RunValidationBlocker>,
) {
    match schema.default_child_group() {
        aivi_gtk::GtkDefaultChildGroup::None if child_count == 0 => {}
        aivi_gtk::GtkDefaultChildGroup::None => blockers.push(RunValidationBlocker {
            span,
            message: format!(
                "`aivi run` does not support child widgets under `{}`; the current widget schema defines no child group for this widget",
                schema.markup_name
            ),
        }),
        aivi_gtk::GtkDefaultChildGroup::One(group) if group.accepts_child_count(child_count) => {}
        aivi_gtk::GtkDefaultChildGroup::One(group) => blockers.push(RunValidationBlocker {
            span,
            message: match group.max_children {
                Some(max_children) => format!(
                    "`aivi run` does not support {child_count} child widget(s) in `{}` group `{}`; this {} group allows at most {max_children}",
                    schema.markup_name,
                    group.name,
                    group.container.label()
                ),
                None => format!(
                    "`aivi run` does not support {child_count} child widget(s) in `{}` group `{}`",
                    schema.markup_name,
                    group.name
                ),
            },
        }),
        aivi_gtk::GtkDefaultChildGroup::Ambiguous if child_count == 0 => {}
        aivi_gtk::GtkDefaultChildGroup::Ambiguous => blockers.push(RunValidationBlocker {
            span,
            message: format!(
                "`aivi run` cannot place unnamed children under `{}` because the widget schema declares multiple child groups",
                schema.markup_name
            ),
        }),
    }
}

fn validate_run_group_children(
    span: SourceSpan,
    child_count: usize,
    group: &aivi_gtk::GtkChildGroupDescriptor,
    blockers: &mut Vec<RunValidationBlocker>,
) {
    if group.accepts_child_count(child_count) {
        return;
    }
    blockers.push(RunValidationBlocker {
        span,
        message: match group.max_children {
            Some(max_children) => format!(
                "`aivi run` does not support {child_count} child widget(s) in named group `{}`; this {} group allows at most {max_children}",
                group.name,
                group.container.label()
            ),
            None => format!(
                "`aivi run` does not support {child_count} child widget(s) in named group `{}`",
                group.name
            ),
        },
    });
}

fn collect_run_root_widgets(
    bridge: &GtkBridgeGraph,
    root: GtkBridgeNodeRef,
) -> Vec<GtkBridgeNodeRef> {
    let mut widgets = Vec::new();
    let mut worklist = vec![root];
    while let Some(node_ref) = worklist.pop() {
        let Some(node) = bridge.node(node_ref.plan) else {
            continue;
        };
        match &node.kind {
            GtkBridgeNodeKind::Widget(_) => widgets.push(node_ref),
            GtkBridgeNodeKind::Group(group) => extend_child_group_roots(&mut worklist, &group.body),
            GtkBridgeNodeKind::Show(show) => extend_child_group_roots(&mut worklist, &show.body),
            GtkBridgeNodeKind::Each(each) => {
                extend_child_group_roots(&mut worklist, &each.item_template);
                if let Some(empty) = &each.empty_branch {
                    extend_child_group_roots(&mut worklist, &empty.body);
                }
            }
            GtkBridgeNodeKind::Empty(empty) => extend_child_group_roots(&mut worklist, &empty.body),
            GtkBridgeNodeKind::Match(match_node) => {
                for case in &match_node.cases {
                    extend_child_group_roots(&mut worklist, &case.body);
                }
            }
            GtkBridgeNodeKind::Case(case) => extend_child_group_roots(&mut worklist, &case.body),
            GtkBridgeNodeKind::Fragment(fragment) => {
                extend_child_group_roots(&mut worklist, &fragment.body)
            }
            GtkBridgeNodeKind::With(with_node) => {
                extend_child_group_roots(&mut worklist, &with_node.body)
            }
        }
    }
    widgets
}

fn extend_child_group_roots(worklist: &mut Vec<GtkBridgeNodeRef>, group: &GtkChildGroup) {
    worklist.extend(group.roots.iter().rev().copied());
}

fn source_location(sources: &SourceDatabase, span: SourceSpan) -> String {
    let file = &sources[span.file()];
    let location = file.line_column(span.span().start());
    format!(
        "{}:{}:{}",
        file.path().display(),
        location.line,
        location.column
    )
}

struct LoweredRunBackendStack {
    core: aivi_core::Module,
    backend: Arc<BackendProgram>,
}

fn lower_runtime_backend_stack_with_items(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    lower_runtime_backend_stack_impl(module, included_items, command_name, true)
}

fn lower_runtime_backend_stack_with_items_fast(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    lower_runtime_backend_stack_impl(module, included_items, command_name, false)
}

fn lower_runtime_backend_stack_impl(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
    validate: bool,
) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_module_with_items(module, included_items).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed core:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_core_module(&core).map_err(|errors| {
            let mut rendered = format!("typed-core validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    let lambda = lower_lambda_module(&core).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_lambda_module(&lambda).map_err(|errors| {
            let mut rendered = format!("typed-lambda validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    let backend = lower_backend_module(&lambda, module).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_program(&backend).map_err(|errors| {
            let mut rendered = format!("backend validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    Ok(LoweredRunBackendStack {
        core,
        backend: Arc::new(backend),
    })
}

fn lower_runtime_backend_stack_with_workspace(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_module_with_workspace(module, workspace_hirs, included_items)
        .map_err(|errors| {
            let mut rendered = format!("failed to lower {command_name} module into typed core:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    validate_core_module(&core).map_err(|errors| {
        let mut rendered = format!("typed-core validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let lambda = lower_lambda_module(&core).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_lambda_module(&lambda).map_err(|errors| {
        let mut rendered = format!("typed-lambda validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let backend = lower_backend_module(&lambda, module).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_program(&backend).map_err(|errors| {
        let mut rendered = format!("backend validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    Ok(LoweredRunBackendStack {
        core,
        backend: Arc::new(backend),
    })
}

fn resolve_run_event_handlers(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    sites: &MarkupRuntimeExprSites,
    bridge: &GtkBridgeGraph,
    runtime_assembly: &HirRuntimeAssembly,
    sources: &SourceDatabase,
) -> Result<BTreeMap<HirExprId, ResolvedRunEventHandler>, String> {
    let mut handlers = BTreeMap::new();
    for node in bridge.nodes() {
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        for event in &widget.event_hooks {
            let resolved = resolve_run_event_handler(
                module,
                workspace_hirs,
                sites,
                runtime_assembly,
                sources,
                &widget.widget,
                event.name.text(),
                event.handler,
            )?;
            handlers.entry(event.handler).or_insert(resolved);
        }
    }
    Ok(handlers)
}

fn resolve_run_event_handler(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    sites: &MarkupRuntimeExprSites,
    runtime_assembly: &HirRuntimeAssembly,
    sources: &SourceDatabase,
    widget: &aivi_hir::NamePath,
    event_name: &str,
    expr: HirExprId,
) -> Result<ResolvedRunEventHandler, String> {
    let location = source_location(sources, module.exprs()[expr].span);
    let Some(event) = lookup_widget_event(widget, event_name) else {
        return Err(format!(
            "event handler at {location} uses unsupported GTK event `{}` on widget `{}`",
            event_name,
            run_widget_name(widget)
        ));
    };
    match &module.exprs()[expr].kind {
        ExprKind::Name(reference) => {
            let resolved = resolve_run_event_signal_target(
                module,
                workspace_hirs,
                runtime_assembly,
                reference,
                &location,
            )?;
            let payload = event.payload;
            if !event_signal_accepts_payload(resolved.inner_payload_type.as_ref(), payload) {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but `{}` on `{}` publishes `{}` and requires `{}`",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                    event_name,
                    run_widget_name(widget),
                    payload.label(),
                    payload.required_signal_type_label()
                ));
            }
            Ok(ResolvedRunEventHandler {
                signal_item: resolved.item_id,
                signal_name: resolved.signal_name,
                signal_input: resolved.signal_input,
                payload: ResolvedRunEventPayload::GtkPayload,
            })
        }
        ExprKind::Apply { callee, arguments } => {
            if arguments.len() != 1 {
                return Err(format!(
                    "event handler at {location} must call a direct signal name with exactly one explicit payload expression"
                ));
            }
            let ExprKind::Name(reference) = &module.exprs()[*callee].kind else {
                return Err(format!(
                    "event handler at {location} must call a direct signal name when providing an explicit payload"
                ));
            };
            let payload_expr =
                arguments.iter().next().copied().expect(
                    "single-argument handler applications should expose a payload expression",
                );
            let resolved = resolve_run_event_signal_target(
                module,
                workspace_hirs,
                runtime_assembly,
                reference,
                &location,
            )?;
            let required_payload = resolved.inner_payload_type.clone().ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} points at signal `{}`, but explicit payload hooks require a known `Signal A` payload type",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                )
            })?;
            let site = sites.get(payload_expr).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} uses payload expression {} without a collected runtime environment",
                    name_path_text(&reference.path),
                    payload_expr.as_raw()
                )
            })?;
            if !site.ty.same_shape(&required_payload) {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but the explicit payload has type `{}` and the signal requires `{}`",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                    site.ty,
                    required_payload
                ));
            }
            Ok(ResolvedRunEventHandler {
                signal_item: resolved.item_id,
                signal_name: resolved.signal_name,
                signal_input: resolved.signal_input,
                payload: ResolvedRunEventPayload::ScopedInput,
            })
        }
        _ => Err(format!(
            "event handler at {location} must be a direct signal name or a direct signal application with one explicit payload"
        )),
    }
}

/// Result of resolving an event signal target, covering both same-module and cross-module cases.
struct EventSignalResolution {
    item_id: aivi_hir::ItemId,
    signal_name: Box<str>,
    signal_input: RuntimeInputHandle,
    /// Inner payload type of `Signal A` (e.g. `Unit`, `Text`), used for payload validation.
    inner_payload_type: Option<GateType>,
}

fn resolve_run_event_signal_target(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    runtime_assembly: &HirRuntimeAssembly,
    reference: &aivi_hir::TermReference,
    location: &str,
) -> Result<EventSignalResolution, String> {
    match reference.resolution.as_ref() {
        aivi_hir::ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            let Item::Signal(signal) = &module.items()[*item_id] else {
                return Err(format!(
                    "event handler `{}` at {location} resolves to a {}, but event hooks require an input-backed signal",
                    name_path_text(&reference.path),
                    hir_item_kind_label(&module.items()[*item_id])
                ));
            };
            let binding = runtime_assembly.signal(*item_id).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} points at signal `{}` without a runtime binding",
                    name_path_text(&reference.path),
                    signal.name.text()
                )
            })?;
            let Some(signal_input) = binding.input() else {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but only direct input signals are publishable from GTK events",
                    name_path_text(&reference.path),
                    signal.name.text()
                ));
            };
            let inner_payload_type = signal_payload_type(module, signal);
            Ok(EventSignalResolution {
                item_id: *item_id,
                signal_name: signal.name.text().into(),
                signal_input,
                inner_payload_type,
            })
        }
        aivi_hir::ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            let Some(import_binding) = module.imports().get(*import_id) else {
                return Err(format!(
                    "event handler `{}` at {location}: import binding not found",
                    name_path_text(&reference.path)
                ));
            };
            let ImportBindingMetadata::Value {
                ty: ImportValueType::Signal(inner_ty),
            } = &import_binding.metadata
            else {
                return Err(format!(
                    "event handler `{}` at {location} resolves to a cross-module import that is not a Signal",
                    name_path_text(&reference.path)
                ));
            };
            let Some(item_id) =
                workspace_import_signal_item(module, workspace_hirs, *import_id, import_binding)
            else {
                return Err(format!(
                    "event handler `{}` at {location}: no runtime binding found for cross-module signal `{}`",
                    name_path_text(&reference.path),
                    import_binding.local_name.text()
                ));
            };
            let binding = runtime_assembly.signal(item_id).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location}: no runtime binding found for cross-module signal `{}`",
                    name_path_text(&reference.path),
                    import_binding.local_name.text()
                )
            })?;
            let Some(signal_input) = binding.input() else {
                return Err(format!(
                    "event handler `{}` at {location}: cross-module signal `{}` is not a publishable input signal",
                    name_path_text(&reference.path),
                    import_binding.local_name.text()
                ));
            };
            let inner_payload_type = import_value_type_to_gate_type(inner_ty);
            Ok(EventSignalResolution {
                item_id,
                signal_name: import_binding.local_name.text().into(),
                signal_input,
                inner_payload_type,
            })
        }
        _ => Err(format!(
            "event handler `{}` at {location} must resolve directly to a signal item",
            name_path_text(&reference.path)
        )),
    }
}

fn workspace_import_signal_item(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    import_id: ImportId,
    import_binding: &aivi_hir::ImportBinding,
) -> Option<aivi_hir::ItemId> {
    let source_module = import_binding
        .source_module
        .as_deref()
        .map(str::to_owned)
        .or_else(|| workspace_import_source_module(module, import_id))?;
    let workspace_exports = workspace_signal_exports(module, workspace_hirs)?;
    workspace_exports
        .get(source_module.as_str())
        .and_then(|exports| exports.get(import_binding.imported_name.text()).copied())
}

fn workspace_signal_exports<'a>(
    entry_module: &HirModule,
    workspace_hirs: &[(&'a str, &'a HirModule)],
) -> Option<BTreeMap<&'a str, BTreeMap<String, aivi_hir::ItemId>>> {
    let entry_item_count = u32::try_from(entry_module.items().iter().count()).ok()?;
    let entry_import_count = u32::try_from(entry_module.imports().iter().count()).ok()?;
    let mut next_origin = entry_item_count + entry_import_count;
    let mut module_offsets = HashMap::new();
    let mut import_to_module = HashMap::new();
    for (name, workspace_module) in workspace_hirs {
        module_offsets.insert(*name, next_origin);
        import_to_module.insert(*name, workspace_import_to_module(workspace_module));
        let item_count = u32::try_from(workspace_module.items().iter().count()).ok()?;
        let import_count = u32::try_from(workspace_module.imports().iter().count()).ok()?;
        next_origin = next_origin
            .saturating_add(item_count)
            .saturating_add(import_count);
    }

    let mut exports = BTreeMap::<&str, BTreeMap<String, aivi_hir::ItemId>>::new();
    for (name, workspace_module) in workspace_hirs {
        let offset = *module_offsets.get(name)?;
        let mut module_exports = BTreeMap::new();
        for (item_id, item) in workspace_module.items().iter() {
            let Item::Signal(signal) = item else {
                continue;
            };
            module_exports.insert(
                signal.name.text().to_owned(),
                aivi_hir::ItemId::from_raw(offset.saturating_add(item_id.as_raw())),
            );
        }
        exports.insert(*name, module_exports);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for (name, workspace_module) in workspace_hirs {
            let Some(source_modules) = import_to_module.get(name) else {
                continue;
            };
            let mut pending = Vec::new();
            for (workspace_import_id, binding) in workspace_module.imports().iter() {
                let ImportBindingMetadata::Value {
                    ty: ImportValueType::Signal(_),
                } = &binding.metadata
                else {
                    continue;
                };
                let Some(source_module) = binding
                    .source_module
                    .as_deref()
                    .or_else(|| source_modules.get(&workspace_import_id).map(|value| value.as_ref()))
                else {
                    continue;
                };
                let Some(source_exports) = exports.get(source_module) else {
                    continue;
                };
                let Some(&item_id) = source_exports.get(binding.imported_name.text()) else {
                    continue;
                };
                pending.push((binding.local_name.text().to_owned(), item_id));
            }
            let Some(module_exports) = exports.get_mut(name) else {
                continue;
            };
            for (local_name, item_id) in pending {
                if module_exports.insert(local_name, item_id).is_none() {
                    changed = true;
                }
            }
        }
    }

    Some(exports)
}

fn workspace_import_source_module(module: &HirModule, import_id: ImportId) -> Option<String> {
    module.items().iter().find_map(|(_, item)| {
        let Item::Use(use_item) = item else {
            return None;
        };
        use_item
            .imports
            .iter()
            .copied()
            .find(|candidate| *candidate == import_id)
            .map(|_| use_item.module.to_string())
    })
}

fn workspace_import_to_module(module: &HirModule) -> HashMap<ImportId, Box<str>> {
    let mut import_to_module = HashMap::new();
    for (_, item) in module.items().iter() {
        let Item::Use(use_item) = item else {
            continue;
        };
        for import_id in use_item.imports.iter().copied() {
            import_to_module.insert(import_id, use_item.module.to_string().into_boxed_str());
        }
    }
    import_to_module
}

/// Checks whether a resolved signal inner type accepts the given GTK event payload.
fn event_signal_accepts_payload(
    inner_ty: Option<&GateType>,
    payload: GtkConcreteEventPayload,
) -> bool {
    let Some(inner_ty) = inner_ty else {
        return false;
    };
    match payload {
        GtkConcreteEventPayload::Unit => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Unit))
        }
        GtkConcreteEventPayload::Bool => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Bool))
        }
        GtkConcreteEventPayload::Text => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Text))
        }
        GtkConcreteEventPayload::F64 => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Float))
        }
        GtkConcreteEventPayload::I64 => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Int))
        }
    }
}

/// Converts an `ImportValueType` to a `GateType` without needing module context.
/// Returns `None` for `Named` types (user-defined type constructors) which require module lookup.
fn import_value_type_to_gate_type(ty: &ImportValueType) -> Option<GateType> {
    Some(match ty {
        ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
        ImportValueType::Tuple(elements) => GateType::Tuple(
            elements
                .iter()
                .filter_map(import_value_type_to_gate_type)
                .collect(),
        ),
        ImportValueType::Record(fields) => GateType::Record(
            fields
                .iter()
                .filter_map(|field| {
                    import_value_type_to_gate_type(&field.ty).map(|ty| GateRecordField {
                        name: field.name.to_string(),
                        ty,
                    })
                })
                .collect(),
        ),
        ImportValueType::Arrow { parameter, result } => GateType::Arrow {
            parameter: Box::new(import_value_type_to_gate_type(parameter)?),
            result: Box::new(import_value_type_to_gate_type(result)?),
        },
        ImportValueType::List(element) => {
            GateType::List(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Map { key, value } => GateType::Map {
            key: Box::new(import_value_type_to_gate_type(key)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Set(element) => {
            GateType::Set(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Option(element) => {
            GateType::Option(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Result { error, value } => GateType::Result {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Validation { error, value } => GateType::Validation {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Signal(element) => {
            GateType::Signal(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Task { error, value } => GateType::Task {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::TypeVariable { index, name } => GateType::TypeParameter {
            parameter: aivi_hir::TypeParameterId::from_raw(u32::MAX - *index as u32),
            name: name.clone(),
        },
        ImportValueType::Named {
            type_name,
            arguments,
            definition,
        } => GateType::OpaqueImport {
            import: aivi_hir::ImportId::from_raw(u32::MAX),
            name: type_name.clone(),
            arguments: arguments
                .iter()
                .map(import_value_type_to_gate_type)
                .collect::<Option<Vec<_>>>()?,
            definition: definition.clone(),
        },
    })
}

fn name_path_text(path: &aivi_hir::NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}

fn run_widget_name(path: &aivi_hir::NamePath) -> &str {
    path.segments()
        .iter()
        .last()
        .expect("NamePath is non-empty")
        .text()
}

fn hir_item_kind_label(item: &Item) -> &'static str {
    match item {
        Item::Type(_) => "type",
        Item::Value(_) => "value",
        Item::Function(_) => "function",
        Item::Signal(_) => "signal",
        Item::Class(_) => "class",
        Item::Domain(_) => "domain",
        Item::SourceProviderContract(_) => "provider",
        Item::Instance(_) => "instance",
        Item::Use(_) => "use",
        Item::Export(_) => "export",
        Item::Hoist(_) => "hoist",
    }
}

fn collect_run_input_specs_from_bridge(
    module: &HirModule,
    bridge: &GtkBridgeGraph,
) -> BTreeMap<RuntimeInputHandle, RunInputSpec> {
    let mut inputs = BTreeMap::new();
    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Widget(widget) => {
                for property in &widget.properties {
                    if let RuntimePropertyBinding::Setter(setter) = property {
                        let spec = match &setter.source {
                            SetterSource::Expr(expr) => RunInputSpec::Expr(*expr),
                            SetterSource::InterpolatedText(text) => {
                                RunInputSpec::Text(text.clone())
                            }
                        };
                        inputs.insert(setter.input, spec);
                    }
                }
                for event in &widget.event_hooks {
                    if let Some(payload_expr) = event_handler_payload_expr(module, event.handler) {
                        inputs.insert(event.input, RunInputSpec::Expr(payload_expr));
                    }
                }
            }
            GtkBridgeNodeKind::Group(_) => {}
            GtkBridgeNodeKind::Show(show) => {
                inputs.insert(show.when.input, RunInputSpec::Expr(show.when.expr));
                if let RuntimeShowMountPolicy::KeepMounted { decision } = &show.mount {
                    inputs.insert(decision.input, RunInputSpec::Expr(decision.expr));
                }
            }
            GtkBridgeNodeKind::Each(each) => {
                inputs.insert(
                    each.collection.input,
                    RunInputSpec::Expr(each.collection.expr),
                );
                if let Some(key_input) = &each.key_input {
                    inputs.insert(key_input.input, RunInputSpec::Expr(key_input.expr));
                }
            }
            GtkBridgeNodeKind::Match(match_node) => {
                inputs.insert(
                    match_node.scrutinee.input,
                    RunInputSpec::Expr(match_node.scrutinee.expr),
                );
            }
            GtkBridgeNodeKind::With(with_node) => {
                inputs.insert(
                    with_node.value.input,
                    RunInputSpec::Expr(with_node.value.expr),
                );
            }
            GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Case(_)
            | GtkBridgeNodeKind::Fragment(_) => {}
        }
    }
    inputs
}

fn compile_run_inputs(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    view_owner: aivi_hir::ItemId,
    sites: &aivi_hir::MarkupRuntimeExprSites,
    bridge: &GtkBridgeGraph,
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
    query_context: Option<BackendQueryContext<'_>>,
) -> Result<
    (
        BTreeMap<RuntimeInputHandle, CompiledRunInput>,
        RunInputCompilationMetrics,
    ),
    String,
> {
    let input_specs = collect_run_input_specs_from_bridge(module, bridge);
    let mut inputs = BTreeMap::new();
    let mut metrics = RunInputCompilationMetrics::default();
    if query_context.is_some() {
        let unique_exprs = collect_unique_run_input_exprs(&input_specs);
        let compile_expr = |expr| {
            compile_run_fragment_for_input(
                module,
                sources,
                workspace_hirs,
                view_owner,
                sites,
                runtime_backend,
                runtime_backend_by_hir,
                query_context,
                expr,
            )
            .map(|fragment| (expr, fragment))
        };
        let parallelism = bounded_fragment_parallelism(unique_exprs.len());
        let batch_size = fragment_batch_size(unique_exprs.len());
        let mut compiled_fragments = BTreeMap::new();
        for expr_batch in unique_exprs.chunks(batch_size) {
            let batch_results = if parallelism <= 1 {
                expr_batch
                    .iter()
                    .copied()
                    .map(compile_expr)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                match fragment_thread_pool() {
                    Some(pool) => pool.install(|| {
                        expr_batch
                            .par_iter()
                            .copied()
                            .map(compile_expr)
                            .collect::<Result<Vec<_>, _>>()
                    })?,
                    None => expr_batch
                        .iter()
                        .copied()
                        .map(compile_expr)
                        .collect::<Result<Vec<_>, _>>()?,
                }
            };
            compiled_fragments.extend(batch_results);
        }
        metrics.compiled_fragment_count = compiled_fragments.len();
        for (input, spec) in input_specs {
            let compiled = match spec {
                RunInputSpec::Expr(expr) => CompiledRunInput::Expr(
                    compiled_fragments
                        .get(&expr)
                        .cloned()
                        .expect("parallel run fragment compilation should cover every unique expr"),
                ),
                RunInputSpec::Text(text) => {
                    let mut segments = Vec::with_capacity(text.segments.len());
                    for segment in text.segments {
                        match segment {
                            aivi_hir::TextSegment::Text(text) => {
                                segments.push(CompiledRunTextSegment::Text(text.raw));
                            }
                            aivi_hir::TextSegment::Interpolation(interpolation) => {
                                segments.push(CompiledRunTextSegment::Interpolation(
                                    compiled_fragments
                                        .get(&interpolation.expr)
                                        .cloned()
                                        .expect("parallel run fragment compilation should cover interpolation exprs"),
                                ));
                            }
                        }
                    }
                    CompiledRunInput::Text(CompiledRunText {
                        segments: segments.into_boxed_slice(),
                    })
                }
            };
            inputs.insert(input, compiled);
        }
        return Ok((inputs, metrics));
    }
    let mut compiler = RunFragmentCompiler::new(
        sources,
        module,
        workspace_hirs,
        view_owner,
        sites,
        runtime_backend,
        runtime_backend_by_hir,
        query_context,
    );
    let mut compile_fragment = |expr| {
        let (fragment, compiled_now) = compiler.compile(expr)?;
        if compiled_now {
            metrics.compiled_fragment_count += 1;
        }
        Ok::<_, String>(fragment)
    };
    for (input, spec) in input_specs {
        let compiled = match spec {
            RunInputSpec::Expr(expr) => CompiledRunInput::Expr(compile_fragment(expr)?),
            RunInputSpec::Text(text) => {
                let mut segments = Vec::with_capacity(text.segments.len());
                for segment in text.segments {
                    match segment {
                        aivi_hir::TextSegment::Text(text) => {
                            segments.push(CompiledRunTextSegment::Text(text.raw));
                        }
                        aivi_hir::TextSegment::Interpolation(interpolation) => {
                            segments.push(CompiledRunTextSegment::Interpolation(compile_fragment(
                                interpolation.expr,
                            )?))
                        }
                    }
                }
                CompiledRunInput::Text(CompiledRunText {
                    segments: segments.into_boxed_slice(),
                })
            }
        };
        inputs.insert(input, compiled);
    }
    Ok((inputs, metrics))
}

fn bounded_fragment_parallelism(work_items: usize) -> usize {
    if work_items <= 1 {
        return 1;
    }
    configured_fragment_parallelism().min(work_items).max(1)
}

fn fragment_batch_size(work_items: usize) -> usize {
    bounded_fragment_parallelism(work_items).saturating_mul(8).max(1)
}

fn configured_fragment_parallelism() -> usize {
    const DEFAULT_MAX_THREADS: usize = 2;
    let available = std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(1);
    let configured = std::env::var("AIVI_FRAGMENT_PARALLELISM")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|threads| *threads > 0)
        .unwrap_or(DEFAULT_MAX_THREADS);
    configured.min(available).max(1)
}

fn fragment_thread_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    if configured_fragment_parallelism() <= 1 {
        return None;
    }
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(configured_fragment_parallelism())
            .build()
            .ok()
    })
    .as_ref()
}

fn collect_unique_run_input_exprs(
    input_specs: &BTreeMap<RuntimeInputHandle, RunInputSpec>,
) -> Vec<HirExprId> {
    let mut exprs = BTreeSet::new();
    for spec in input_specs.values() {
        match spec {
            RunInputSpec::Expr(expr) => {
                exprs.insert(*expr);
            }
            RunInputSpec::Text(text) => {
                for segment in &text.segments {
                    if let aivi_hir::TextSegment::Interpolation(interpolation) = segment {
                        exprs.insert(interpolation.expr);
                    }
                }
            }
        }
    }
    exprs.into_iter().collect()
}
