use std::collections::HashSet;

use aivi_typing::SourceOptionWakeupCause;

use crate::{
    ApplicativeSpineHead, ClusterId, ControlNode, ControlNodeId, CustomSourceContractMetadata,
    DecoratorPayload, ExprId, ExprKind, ImportBindingMetadata, ImportId, ImportValueType, Item,
    ItemId, MarkupAttributeValue, MarkupNodeId, MarkupNodeKind, Module, PatternId, PatternKind,
    ProjectionBase, ResolutionState, SignalItem, SourceDecorator, SourceLifecycleDependencies,
    SourceMetadata, SourceProviderRef, TermResolution, TextSegment,
};

/// Populates `signal_dependencies` and `source_metadata` on every
/// [`Item::Signal`] in `module`. This is a post-resolution elaboration pass:
/// name references must already be resolved before calling this function.
pub fn populate_signal_metadata(module: &mut Module) {
    let item_ids = module
        .items()
        .iter()
        .map(|(item_id, _)| item_id)
        .collect::<Vec<_>>();
    for item_id in item_ids {
        let (signal_dependencies, import_signal_dependencies, source_metadata) =
            match &module.items()[item_id] {
                Item::Signal(item) => compute_signal_metadata(module, item),
                _ => continue,
            };
        let temporal_input_dependencies = match &module.items()[item_id] {
            Item::Signal(item) => compute_temporal_input_dependencies(module, item),
            _ => Vec::new(),
        };
        let Some(Item::Signal(item)) = module.arenas.items.get_mut(item_id) else {
            continue;
        };
        item.signal_dependencies = signal_dependencies;
        item.import_signal_dependencies = import_signal_dependencies;
        item.temporal_input_dependencies = temporal_input_dependencies;
        item.source_metadata = source_metadata;
    }
}

pub fn collect_signal_dependencies_for_expr(module: &Module, expr: ExprId) -> Vec<ItemId> {
    collect_signal_dependencies(module, vec![DependencyWork::Expr(expr)])
}

pub fn collect_signal_dependencies_for_exprs(
    module: &Module,
    exprs: impl IntoIterator<Item = ExprId>,
) -> Vec<ItemId> {
    collect_signal_dependencies(
        module,
        exprs.into_iter().map(DependencyWork::Expr).collect(),
    )
}

pub(crate) fn expr_signal_dependencies<I>(module: &Module, roots: I) -> Vec<ItemId>
where
    I: IntoIterator<Item = ExprId>,
{
    collect_signal_dependencies_for_exprs(module, roots)
}

fn compute_signal_metadata(
    module: &Module,
    item: &SignalItem,
) -> (Vec<ItemId>, Vec<ImportId>, Option<SourceMetadata>) {
    if item.is_source_capability_handle {
        return (Vec::new(), Vec::new(), None);
    }
    let source = item.header.decorators.iter().find_map(|decorator_id| {
        let decorator = &module.decorators()[*decorator_id];
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some(source),
            _ => None,
        }
    });
    let mut work = Vec::new();
    if let Some(body) = item.body {
        work.push(DependencyWork::Expr(body));
    }
    for update in &item.reactive_updates {
        work.push(DependencyWork::Expr(update.guard));
        work.push(DependencyWork::Expr(update.body));
    }
    let source_dependencies = source.map(|source| {
        let mut roots = source
            .arguments
            .iter()
            .copied()
            .map(DependencyWork::Expr)
            .collect::<Vec<_>>();
        if let Some(options) = source.options {
            roots.push(DependencyWork::Expr(options));
        }
        collect_all_signal_dependencies(module, roots)
    });
    let (mut signal_dependencies, mut import_signal_dependencies) =
        collect_all_signal_dependencies(module, work);
    if let Some((source_signal_dependencies, source_import_signal_dependencies)) =
        source_dependencies.as_ref()
    {
        signal_dependencies.extend(source_signal_dependencies.iter().copied());
        import_signal_dependencies.extend(source_import_signal_dependencies.iter().copied());
        normalize_dependency_list(&mut signal_dependencies);
        normalize_import_dependency_list(&mut import_signal_dependencies);
    }
    let source_metadata = source.map(|source| {
        let source_dependencies = source_dependencies.unwrap_or_default();
        let provider = SourceProviderRef::from_path(source.provider.as_ref());
        SourceMetadata {
            custom_contract: resolve_custom_source_contract(module, &provider),
            lifecycle_dependencies: compute_source_lifecycle_dependencies(
                module, source, &provider,
            ),
            provider,
            is_reactive: !source_dependencies.0.is_empty() || !source_dependencies.1.is_empty(),
            signal_dependencies: source_dependencies.0,
        }
    });
    (
        signal_dependencies,
        import_signal_dependencies,
        source_metadata,
    )
}

/// Identifies signal dependencies that flow through a temporal pipe stage
/// (delay or burst). These stages dispatch work to an async worker thread,
/// making them a genuine asynchronous boundary — analogous to `@source`
/// lifecycle dependencies — and safe to exclude from cycle detection.
fn compute_temporal_input_dependencies(module: &Module, item: &SignalItem) -> Vec<ItemId> {
    let Some(body) = item.body else {
        return Vec::new();
    };
    let ExprKind::Pipe(pipe) = &module.exprs()[body].kind else {
        return Vec::new();
    };
    let has_temporal = pipe.stages.iter().any(|stage| stage.is_temporal_boundary());
    if !has_temporal {
        return Vec::new();
    }
    // Collect signal dependencies from the pipe head and all stages before the
    // first temporal boundary. These dependencies feed into the async worker
    // and do not participate in synchronous evaluation order.
    let mut work = vec![DependencyWork::Expr(pipe.head)];
    for stage in pipe.stages.iter() {
        if stage.is_temporal_boundary() {
            break;
        }
        for pattern in stage.pattern_inputs() {
            work.push(DependencyWork::Pattern(pattern.pattern));
        }
        for input in stage.expr_inputs() {
            work.push(DependencyWork::Expr(input.expr));
        }
    }
    collect_signal_dependencies(module, work)
}

fn compute_source_lifecycle_dependencies(
    module: &Module,
    source: &SourceDecorator,
    provider: &SourceProviderRef,
) -> SourceLifecycleDependencies {
    let mut lifecycle = SourceLifecycleDependencies::default();
    lifecycle
        .reconfiguration
        .extend(collect_signal_dependencies(
            module,
            source
                .arguments
                .iter()
                .copied()
                .map(DependencyWork::Expr)
                .collect(),
        ));

    let Some(options) = source.options else {
        normalize_dependency_list(&mut lifecycle.reconfiguration);
        return lifecycle;
    };
    let option_work = vec![DependencyWork::Expr(options)];
    let Some(builtin_provider) = provider.builtin() else {
        lifecycle
            .reconfiguration
            .extend(collect_signal_dependencies(module, option_work));
        normalize_dependency_list(&mut lifecycle.reconfiguration);
        return lifecycle;
    };
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        lifecycle
            .reconfiguration
            .extend(collect_signal_dependencies(module, option_work));
        normalize_dependency_list(&mut lifecycle.reconfiguration);
        return lifecycle;
    };

    let contract = builtin_provider.contract();
    for field in &record.fields {
        let dependencies =
            collect_signal_dependencies(module, vec![DependencyWork::Expr(field.value)]);
        if field.label.text() == "activeWhen" && contract.option("activeWhen").is_some() {
            lifecycle.active_when.extend(dependencies);
            continue;
        }
        match contract
            .wakeup_option(field.label.text())
            .map(|option| option.cause())
        {
            Some(SourceOptionWakeupCause::TriggerSignal) => {
                lifecycle.explicit_triggers.extend(dependencies)
            }
            Some(SourceOptionWakeupCause::RetryPolicy | SourceOptionWakeupCause::PollingPolicy)
            | None => lifecycle.reconfiguration.extend(dependencies),
        }
    }

    normalize_dependency_list(&mut lifecycle.reconfiguration);
    normalize_dependency_list(&mut lifecycle.explicit_triggers);
    normalize_dependency_list(&mut lifecycle.active_when);
    lifecycle
}

/// Resolves the custom [`SourceProviderContract`] item for `provider` by
/// iterating the module's items, avoiding the dependency on the private
/// `Namespaces` struct used during lowering.
fn resolve_custom_source_contract(
    module: &Module,
    provider: &SourceProviderRef,
) -> Option<CustomSourceContractMetadata> {
    let key = provider.custom_key()?;
    crate::custom_source_capabilities::resolve_custom_source_binding_contract(module, key)
}

#[derive(Clone, Copy)]
enum DependencyWork {
    Expr(ExprId),
    Pattern(PatternId),
    Markup(MarkupNodeId),
    Control(ControlNodeId),
    Cluster(ClusterId),
    Item(ItemId),
}

/// Collects both item-based and import-based signal dependencies from expressions.
/// Returns `(item_signal_deps, import_signal_deps)`.
fn collect_all_signal_dependencies(
    module: &Module,
    work: Vec<DependencyWork>,
) -> (Vec<ItemId>, Vec<ImportId>) {
    let (item_deps, import_deps) = collect_signal_deps_internal(module, work);
    let mut item_deps: Vec<_> = item_deps.into_iter().collect();
    item_deps.sort();
    let mut import_deps: Vec<_> = import_deps.into_iter().collect();
    import_deps.sort();
    (item_deps, import_deps)
}

fn collect_signal_dependencies(module: &Module, work: Vec<DependencyWork>) -> Vec<ItemId> {
    let (item_deps, _) = collect_signal_deps_internal(module, work);
    let mut signal_dependencies: Vec<_> = item_deps.into_iter().collect();
    signal_dependencies.sort();
    signal_dependencies
}

/// Core traversal: returns (item_signal_deps, import_signal_deps).
fn collect_signal_deps_internal(
    module: &Module,
    mut work: Vec<DependencyWork>,
) -> (HashSet<ItemId>, HashSet<ImportId>) {
    let mut item_deps: HashSet<ItemId> = HashSet::new();
    let mut import_deps: HashSet<ImportId> = HashSet::new();
    let mut seen_exprs = HashSet::new();
    let mut seen_patterns = HashSet::new();
    let mut seen_markups = HashSet::new();
    let mut seen_controls = HashSet::new();
    let mut seen_clusters = HashSet::new();
    let mut seen_items = HashSet::new();

    while let Some(node) = work.pop() {
        match node {
            DependencyWork::Expr(expr_id) => {
                if !seen_exprs.insert(expr_id) {
                    continue;
                }
                match &module.exprs()[expr_id].kind {
                    ExprKind::Name(reference) => match &reference.resolution {
                        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                            match &module.items()[*item_id] {
                                Item::Signal(_) => {
                                    item_deps.insert(*item_id);
                                }
                                Item::Value(_) | Item::Function(_) => {
                                    work.push(DependencyWork::Item(*item_id));
                                }
                                _ => {}
                            }
                        }
                        ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                            let import = &module.imports()[*import_id];
                            if matches!(
                                &import.metadata,
                                ImportBindingMetadata::Value {
                                    ty: ImportValueType::Signal(_)
                                }
                            ) {
                                import_deps.insert(*import_id);
                            }
                        }
                        _ => {}
                    },
                    ExprKind::Integer(_)
                    | ExprKind::Float(_)
                    | ExprKind::Decimal(_)
                    | ExprKind::BigInt(_)
                    | ExprKind::SuffixedInteger(_)
                    | ExprKind::AmbientSubject
                    | ExprKind::Regex(_) => {}
                    ExprKind::Text(text) => {
                        for segment in &text.segments {
                            if let TextSegment::Interpolation(interpolation) = segment {
                                work.push(DependencyWork::Expr(interpolation.expr));
                            }
                        }
                    }
                    ExprKind::Tuple(elements) => {
                        work.extend(elements.iter().copied().map(DependencyWork::Expr));
                    }
                    ExprKind::List(elements) => {
                        work.extend(elements.iter().copied().map(DependencyWork::Expr));
                    }
                    ExprKind::Map(map) => {
                        work.extend(map.entries.iter().flat_map(|entry| {
                            [
                                DependencyWork::Expr(entry.key),
                                DependencyWork::Expr(entry.value),
                            ]
                        }));
                    }
                    ExprKind::Set(elements) => {
                        work.extend(elements.iter().copied().map(DependencyWork::Expr));
                    }
                    ExprKind::Lambda(lambda) => {
                        work.push(DependencyWork::Expr(lambda.body));
                    }
                    ExprKind::Record(record) => {
                        work.extend(
                            record
                                .fields
                                .iter()
                                .map(|field| DependencyWork::Expr(field.value)),
                        );
                    }
                    ExprKind::Projection { base, .. } => {
                        if let ProjectionBase::Expr(base) = base {
                            work.push(DependencyWork::Expr(*base));
                        }
                    }
                    ExprKind::Apply { callee, arguments } => {
                        work.push(DependencyWork::Expr(*callee));
                        work.extend(arguments.iter().copied().map(DependencyWork::Expr));
                    }
                    ExprKind::Unary { expr, .. } => work.push(DependencyWork::Expr(*expr)),
                    ExprKind::Binary { left, right, .. } => {
                        work.push(DependencyWork::Expr(*left));
                        work.push(DependencyWork::Expr(*right));
                    }
                    ExprKind::PatchApply { target, patch } => {
                        work.push(DependencyWork::Expr(*target));
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(DependencyWork::Expr(expr))
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(DependencyWork::Expr(*expr));
                                }
                            }
                        }
                    }
                    ExprKind::PatchLiteral(patch) => {
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(DependencyWork::Expr(expr))
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(DependencyWork::Expr(*expr));
                                }
                            }
                        }
                    }
                    ExprKind::Pipe(pipe) => {
                        work.push(DependencyWork::Expr(pipe.head));
                        for stage in pipe.stages.iter() {
                            for input in stage.expr_inputs() {
                                work.push(DependencyWork::Expr(input.expr));
                            }
                            for pattern in stage.pattern_inputs() {
                                work.push(DependencyWork::Pattern(pattern.pattern));
                            }
                        }
                    }
                    ExprKind::Cluster(cluster_id) => {
                        work.push(DependencyWork::Cluster(*cluster_id))
                    }
                    ExprKind::Markup(node_id) => work.push(DependencyWork::Markup(*node_id)),
                }
            }
            DependencyWork::Pattern(pattern_id) => {
                if !seen_patterns.insert(pattern_id) {
                    continue;
                }
                match &module.patterns()[pattern_id].kind {
                    PatternKind::Wildcard
                    | PatternKind::Binding(_)
                    | PatternKind::Integer(_)
                    | PatternKind::UnresolvedName(_) => {}
                    PatternKind::Text(text) => {
                        for segment in &text.segments {
                            if let TextSegment::Interpolation(interpolation) = segment {
                                work.push(DependencyWork::Expr(interpolation.expr));
                            }
                        }
                    }
                    PatternKind::Tuple(elements) => {
                        work.extend(elements.iter().copied().map(DependencyWork::Pattern));
                    }
                    PatternKind::List { elements, rest } => {
                        work.extend(elements.iter().copied().map(DependencyWork::Pattern));
                        if let Some(rest) = rest {
                            work.push(DependencyWork::Pattern(*rest));
                        }
                    }
                    PatternKind::Record(fields) => {
                        work.extend(
                            fields
                                .iter()
                                .map(|field| DependencyWork::Pattern(field.pattern)),
                        );
                    }
                    PatternKind::Constructor { arguments, .. } => {
                        work.extend(arguments.iter().copied().map(DependencyWork::Pattern));
                    }
                }
            }
            DependencyWork::Markup(node_id) => {
                if !seen_markups.insert(node_id) {
                    continue;
                }
                match &module.markup_nodes()[node_id].kind {
                    MarkupNodeKind::Element(element) => {
                        for attribute in &element.attributes {
                            match &attribute.value {
                                MarkupAttributeValue::ImplicitTrue => {}
                                MarkupAttributeValue::Expr(expr) => {
                                    work.push(DependencyWork::Expr(*expr))
                                }
                                MarkupAttributeValue::Text(text) => {
                                    for segment in &text.segments {
                                        if let TextSegment::Interpolation(interpolation) = segment {
                                            work.push(DependencyWork::Expr(interpolation.expr));
                                        }
                                    }
                                }
                            }
                        }
                        work.extend(element.children.iter().copied().map(DependencyWork::Markup));
                    }
                    MarkupNodeKind::Control(control_id) => {
                        work.push(DependencyWork::Control(*control_id))
                    }
                }
            }
            DependencyWork::Control(control_id) => {
                if !seen_controls.insert(control_id) {
                    continue;
                }
                match &module.control_nodes()[control_id] {
                    ControlNode::Show(show) => {
                        work.push(DependencyWork::Expr(show.when));
                        if let Some(keep_mounted) = show.keep_mounted {
                            work.push(DependencyWork::Expr(keep_mounted));
                        }
                        work.extend(show.children.iter().copied().map(DependencyWork::Markup));
                    }
                    ControlNode::Each(each) => {
                        work.push(DependencyWork::Expr(each.collection));
                        if let Some(key) = each.key {
                            work.push(DependencyWork::Expr(key));
                        }
                        work.extend(each.children.iter().copied().map(DependencyWork::Markup));
                        if let Some(empty) = each.empty {
                            work.push(DependencyWork::Control(empty));
                        }
                    }
                    ControlNode::Empty(empty) => {
                        work.extend(empty.children.iter().copied().map(DependencyWork::Markup));
                    }
                    ControlNode::Match(match_node) => {
                        work.push(DependencyWork::Expr(match_node.scrutinee));
                        work.extend(
                            match_node
                                .cases
                                .iter()
                                .copied()
                                .map(DependencyWork::Control),
                        );
                    }
                    ControlNode::Case(case) => {
                        work.push(DependencyWork::Pattern(case.pattern));
                        work.extend(case.children.iter().copied().map(DependencyWork::Markup));
                    }
                    ControlNode::Fragment(fragment) => {
                        work.extend(
                            fragment
                                .children
                                .iter()
                                .copied()
                                .map(DependencyWork::Markup),
                        );
                    }
                    ControlNode::With(with) => {
                        work.push(DependencyWork::Expr(with.value));
                        work.extend(with.children.iter().copied().map(DependencyWork::Markup));
                    }
                }
            }
            DependencyWork::Cluster(cluster_id) => {
                if !seen_clusters.insert(cluster_id) {
                    continue;
                }
                let cluster = &module.clusters()[cluster_id];
                let spine = cluster.normalized_spine();
                work.extend(spine.apply_arguments().map(DependencyWork::Expr));
                if let ApplicativeSpineHead::Expr(expr) = spine.pure_head() {
                    work.push(DependencyWork::Expr(expr));
                }
            }
            DependencyWork::Item(item_id) => {
                if !seen_items.insert(item_id) {
                    continue;
                }
                match &module.items()[item_id] {
                    Item::Value(item) => work.push(DependencyWork::Expr(item.body)),
                    Item::Function(item) => work.push(DependencyWork::Expr(item.body)),
                    Item::Type(_)
                    | Item::Signal(_)
                    | Item::Class(_)
                    | Item::Domain(_)
                    | Item::SourceProviderContract(_)
                    | Item::Instance(_)
                    | Item::Use(_)
                    | Item::Export(_)
                    | Item::Hoist(_) => {}
                }
            }
        }
    }

    (item_deps, import_deps)
}

fn normalize_dependency_list(dependencies: &mut Vec<ItemId>) {
    dependencies.sort();
    dependencies.dedup();
}

fn normalize_import_dependency_list(dependencies: &mut Vec<ImportId>) {
    dependencies.sort();
    dependencies.dedup();
}
