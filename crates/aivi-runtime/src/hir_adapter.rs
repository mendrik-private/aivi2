use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use aivi_backend::{
    CommittedValueStore, InlineCommittedValueStore, ItemId as BackendItemId,
    Program as BackendProgram, lower_module_with_hir as lower_backend_module, validate_program,
};
use aivi_base::SourceSpan;
use aivi_core::{RuntimeFragmentSpec, lower_runtime_fragment};
use aivi_hir as hir;
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_typing::RecurrenceTarget;

use crate::{
    effects::{
        RuntimeSourceProvider, SourceInstanceId, SourceReplacementPolicy, SourceRuntimeSpec,
        SourceStaleWorkPolicy, TaskInstanceId, TaskRuntimeSpec, TaskSourceRuntime,
        TaskSourceRuntimeError,
    },
    graph::{
        DerivedHandle, GraphBuildError, InputHandle, OwnerHandle, ReactiveClauseBuilderSpec,
        ReactiveClauseHandle, SignalGraph, SignalGraphBuilder, SignalHandle,
    },
    reactive_program::{ReactiveProgram, build_reactive_program},
};

/// Build a concrete runtime assembly directly from the current HIR handoffs.
///
/// The current compiler wave already exposes source lifecycle plans, decode programs, gate plans,
/// and recurrence plans, but it does not yet define full evaluation/codegen semantics for every
/// signal body. This adapter therefore chooses the narrowest coherent runtime boundary:
///
/// - every `sig` with a body becomes one public runtime signal node,
/// - every `@source` contributes one concrete source runtime spec and publication input,
/// - every directly annotated top-level `val ... : Task E A` contributes one concrete task
///   runtime spec plus a dedicated scheduler-owned sink input,
/// - source-decorated signals with bodies keep those identities separate so the unresolved
///   publication-to-body lowering gap stays explicit,
/// - and gate/recurrence plans are preserved as typed runtime-facing attachments rather than being
///   re-derived from raw HIR later.
///
/// This keeps the compiler/runtime seam honest, explicit, and testable without inventing execution
/// semantics the current frontend has not justified yet.
pub fn assemble_hir_runtime(
    module: &hir::Module,
) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
    let source_lifecycles = hir::elaborate_source_lifecycles(module);
    let source_decode_programs = hir::generate_source_decode_programs(module);
    let recurrences = hir::elaborate_recurrences(module);
    let gates = hir::elaborate_gates(module);
    HirRuntimeAssemblyBuilder::new(
        module,
        &source_lifecycles,
        &source_decode_programs,
        &recurrences,
        &gates,
    )
    .build()
}

pub fn assemble_hir_runtime_with_items(
    module: &hir::Module,
    included_items: &HashSet<hir::ItemId>,
) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
    let source_lifecycles = hir::elaborate_source_lifecycles(module);
    let source_decode_programs = hir::generate_source_decode_programs(module);
    let recurrences = hir::elaborate_recurrences(module);
    let gates = hir::elaborate_gates(module);
    HirRuntimeAssemblyBuilder::new_with_items(
        module,
        &source_lifecycles,
        &source_decode_programs,
        &recurrences,
        &gates,
        included_items,
    )
    .build()
}

pub fn assemble_hir_runtime_with_items_profiled(
    module: &hir::Module,
    included_items: &HashSet<hir::ItemId>,
) -> Result<ProfiledHirRuntimeAssembly, HirRuntimeAdapterErrors> {
    let source_lifecycles = hir::elaborate_source_lifecycles(module);
    let source_decode_programs = hir::generate_source_decode_programs(module);
    let recurrences = hir::elaborate_recurrences(module);
    let gates = hir::elaborate_gates(module);
    HirRuntimeAssemblyBuilder::new_with_items(
        module,
        &source_lifecycles,
        &source_decode_programs,
        &recurrences,
        &gates,
        included_items,
    )
    .build_profiled()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HirRuntimeAssemblyStats {
    pub reactive_guard_fragments: usize,
    pub reactive_body_fragments: usize,
    pub reactive_fragment_compile_duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfiledHirRuntimeAssembly {
    pub assembly: HirRuntimeAssembly,
    pub stats: HirRuntimeAssemblyStats,
}

#[derive(Clone, Debug)]
pub struct HirRuntimeAssemblyBuilder<'a> {
    module: &'a hir::Module,
    source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
    source_decode_programs: &'a hir::SourceDecodeProgramReport,
    recurrences: &'a hir::RecurrenceElaborationReport,
    gates: &'a hir::GateElaborationReport,
    included_items: Option<&'a HashSet<hir::ItemId>>,
}

impl<'a> HirRuntimeAssemblyBuilder<'a> {
    pub const fn new(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
    ) -> Self {
        Self::new_internal(
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            None,
        )
    }

    pub const fn new_with_items(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
        included_items: &'a HashSet<hir::ItemId>,
    ) -> Self {
        Self::new_internal(
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            Some(included_items),
        )
    }

    const fn new_internal(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
        included_items: Option<&'a HashSet<hir::ItemId>>,
    ) -> Self {
        Self {
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            included_items,
        }
    }

    fn includes_item(&self, item: hir::ItemId) -> bool {
        self.included_items
            .as_ref()
            .is_none_or(|included| included.contains(&item))
    }

    pub fn build(self) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
        self.build_profiled().map(|profiled| profiled.assembly)
    }

    pub fn build_profiled(self) -> Result<ProfiledHirRuntimeAssembly, HirRuntimeAdapterErrors> {
        let mut errors = Vec::new();
        let mut stats = HirRuntimeAssemblyStats::default();
        let report_index = ReportIndex::new(
            self.source_lifecycles,
            self.source_decode_programs,
            &mut errors,
        );
        let mut graph_builder = SignalGraphBuilder::new();
        let mut owners = Vec::new();
        let mut signals = Vec::new();
        let mut public_signals = BTreeMap::<hir::ItemId, SignalHandle>::new();
        let mut public_signal_names = BTreeMap::<String, SignalHandle>::new();
        let mut source_inputs = BTreeMap::<hir::ItemId, InputHandle>::new();

        for (item_id, item) in self.module.items().iter() {
            if !self.includes_item(item_id) {
                continue;
            }
            let hir::Item::Signal(signal) = item else {
                continue;
            };
            if signal.is_source_capability_handle {
                continue;
            }
            let has_source = signal.source_metadata.is_some();
            let has_body = signal.body.is_some();
            let has_reactive_updates = !signal.reactive_updates.is_empty();

            let owner = match graph_builder.add_owner(signal.name.text(), None) {
                Ok(owner) => owner,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            owners.push(HirOwnerBinding {
                item: item_id,
                span: signal.header.span,
                name: signal.name.text().into(),
                handle: owner,
            });

            let (kind, source_input) = if has_reactive_updates {
                let reactive = match graph_builder.add_reactive(signal.name.text(), Some(owner)) {
                    Ok(signal) => signal,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                let source_input = if has_source {
                    match graph_builder
                        .add_input(format!("{}#source", signal.name.text()), Some(owner))
                    {
                        Ok(input) => Some(input),
                        Err(err) => {
                            errors.push(HirRuntimeAdapterError::GraphBuild(err));
                            continue;
                        }
                    }
                } else {
                    None
                };
                public_signals.insert(item_id, reactive);
                public_signal_names.insert(signal.name.text().to_owned(), reactive);
                (
                    HirSignalBindingKind::Reactive {
                        signal: reactive,
                        dependencies: Vec::new().into_boxed_slice(),
                        seed_dependencies: Vec::new().into_boxed_slice(),
                        clauses: Vec::new().into_boxed_slice(),
                    },
                    source_input,
                )
            } else if has_body {
                let derived = match graph_builder.add_derived(signal.name.text(), Some(owner)) {
                    Ok(derived) => derived,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                let source_input = if has_source {
                    match graph_builder
                        .add_input(format!("{}#source", signal.name.text()), Some(owner))
                    {
                        Ok(input) => Some(input),
                        Err(err) => {
                            errors.push(HirRuntimeAdapterError::GraphBuild(err));
                            continue;
                        }
                    }
                } else {
                    None
                };
                let temporal_helpers = match signal.body {
                    Some(body) => add_temporal_helper_inputs(
                        self.module,
                        &mut graph_builder,
                        signal.name.text(),
                        owner,
                        body,
                    ),
                    None => Ok(Vec::new().into_boxed_slice()),
                };
                let temporal_helpers = match temporal_helpers {
                    Ok(helpers) => helpers,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                public_signals.insert(item_id, derived.as_signal());
                public_signal_names.insert(signal.name.text().to_owned(), derived.as_signal());
                (
                    HirSignalBindingKind::Derived {
                        signal: derived,
                        dependencies: Vec::new().into_boxed_slice(),
                        temporal_trigger_dependencies: Vec::new().into_boxed_slice(),
                        temporal_helpers,
                    },
                    source_input,
                )
            } else {
                let input = match graph_builder.add_input(signal.name.text(), Some(owner)) {
                    Ok(input) => input,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                public_signals.insert(item_id, input.as_signal());
                public_signal_names.insert(signal.name.text().to_owned(), input.as_signal());
                (HirSignalBindingKind::Input { signal: input }, Some(input))
            };

            if let Some(input) = source_input {
                source_inputs.insert(item_id, input);
            }
            signals.push(HirSignalBinding {
                item: item_id,
                span: signal.header.span,
                name: signal.name.text().into(),
                owner,
                kind,
                source_input,
            });
        }

        // Add stub Input signal entries for all workspace Signal import bindings.
        // These use the same deterministic synthetic HirItemId formula as lower.rs:
        //   synthetic_id = hir_item_count + import_id.as_raw()
        // This ensures signal_items_by_handle can map signal handles → backend items.
        let hir_item_count =
            u32::try_from(self.module.items().iter().count()).expect("HIR item count fits u32");
        for (import_id, binding) in self.module.imports().iter() {
            let hir::ImportBindingMetadata::Value {
                ty: hir::ImportValueType::Signal(_),
            } = &binding.metadata
            else {
                continue;
            };
            let signal_name = binding.local_name.text();
            let synthetic_item_id = hir::ItemId::from_raw(hir_item_count + import_id.as_raw());
            let owner = match graph_builder.add_owner(signal_name, None) {
                Ok(owner) => owner,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            let input = match graph_builder.add_input(signal_name, Some(owner)) {
                Ok(input) => input,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            public_signals.insert(synthetic_item_id, input.as_signal());
            public_signal_names.insert(signal_name.to_owned(), input.as_signal());
            source_inputs.insert(synthetic_item_id, input);
            owners.push(HirOwnerBinding {
                item: synthetic_item_id,
                span: binding.span,
                name: signal_name.into(),
                handle: owner,
            });
            signals.push(HirSignalBinding {
                item: synthetic_item_id,
                span: binding.span,
                name: signal_name.into(),
                owner,
                kind: HirSignalBindingKind::Input { signal: input },
                source_input: Some(input),
            });
        }

        let mut next_reactive_clause_raw = 0u32;
        for binding in &mut signals {
            let Some(hir::Item::Signal(signal)) = self.module.items().get(binding.item) else {
                continue;
            };
            match &mut binding.kind {
                HirSignalBindingKind::Input { .. } => {}
                HirSignalBindingKind::Derived {
                    signal: derived,
                    dependencies,
                    temporal_trigger_dependencies,
                    temporal_helpers,
                } => {
                    let mut resolved = resolve_signal_dependencies(
                        self.module,
                        binding.item,
                        &signal.signal_dependencies,
                        &public_signals,
                        &mut errors,
                    );
                    let mut resolved_temporal_triggers = resolve_signal_dependencies(
                        self.module,
                        binding.item,
                        &signal.temporal_input_dependencies,
                        &public_signals,
                        &mut errors,
                    );
                    // Resolve imported signal dependencies (workspace module signals
                    // referenced via TermResolution::Import). These use the same
                    // synthetic ItemId formula as the import stub block above.
                    for &import_id in &signal.import_signal_dependencies {
                        let synthetic_id =
                            hir::ItemId::from_raw(hir_item_count + import_id.as_raw());
                        match public_signals.get(&synthetic_id).copied() {
                            Some(handle) => push_unique_signal(&mut resolved, handle),
                            None => errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                                owner: binding.item,
                                dependency: synthetic_id,
                            }),
                        }
                    }
                    let mut graph_dependencies = resolved
                        .iter()
                        .copied()
                        .filter(|dependency| !resolved_temporal_triggers.contains(dependency))
                        .collect::<Vec<_>>();
                    if let Some(source_input) = binding.source_input {
                        if temporal_helpers.is_empty() {
                            push_unique_signal(&mut graph_dependencies, source_input.as_signal());
                        } else {
                            push_unique_signal(
                                &mut resolved_temporal_triggers,
                                source_input.as_signal(),
                            );
                        }
                    }
                    for &helper in temporal_helpers.iter() {
                        push_unique_signal(&mut graph_dependencies, helper.as_signal());
                    }
                    if let Err(err) =
                        graph_builder.define_derived(*derived, graph_dependencies.iter().copied())
                    {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                    *dependencies = graph_dependencies.into_boxed_slice();
                    *temporal_trigger_dependencies = resolved_temporal_triggers.into_boxed_slice();
                }
                HirSignalBindingKind::Reactive {
                    signal: reactive,
                    dependencies,
                    seed_dependencies,
                    clauses,
                } => {
                    let signal_pipeline_dependencies = signal
                        .body
                        .map(|body| {
                            collect_pipe_stage_signal_dependencies(
                                self.module,
                                body,
                                &public_signal_names,
                            )
                        })
                        .unwrap_or_default();
                    let mut resolved_seed_dependencies = signal
                        .body
                        .map(|body| {
                            let mut dependencies = resolve_signal_dependencies(
                                self.module,
                                binding.item,
                                &hir::collect_signal_dependencies_for_expr(self.module, body),
                                &public_signals,
                                &mut errors,
                            );
                            if dependencies.is_empty() {
                                dependencies = collect_direct_signal_dependencies(
                                    self.module,
                                    body,
                                    &public_signal_names,
                                );
                            }
                            dependencies
                        })
                        .unwrap_or_default();
                    let mut reactive_dependencies = signal_pipeline_dependencies.clone();
                    for dependency in &resolved_seed_dependencies {
                        push_unique_signal(&mut reactive_dependencies, *dependency);
                    }

                    let payload_type = hir::signal_payload_type(self.module, signal);
                    let mut clause_bindings = Vec::with_capacity(signal.reactive_updates.len());
                    let mut clause_specs = Vec::with_capacity(signal.reactive_updates.len());
                    for (clause_index, update) in signal.reactive_updates.iter().enumerate() {
                        let trigger_signal = match update.trigger_source {
                            Some(source_item) => match public_signals.get(&source_item).copied() {
                                Some(signal) => Some(signal),
                                None => {
                                    errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                                        owner: binding.item,
                                        dependency: source_item,
                                    });
                                    continue;
                                }
                            },
                            None => None,
                        };
                        let Some(payload_type) = payload_type.as_ref() else {
                            errors.push(HirRuntimeAdapterError::ReactiveUpdateUnknownPayloadType {
                                owner: binding.item,
                                clause_span: update.span,
                            });
                            continue;
                        };
                        let bool_type = hir::GateType::Primitive(hir::BuiltinType::Bool);
                        let body_type = match update.body_mode {
                            hir::ReactiveUpdateBodyMode::Payload => payload_type.clone(),
                            hir::ReactiveUpdateBodyMode::OptionalPayload => {
                                hir::GateType::Option(Box::new(payload_type.clone()))
                            }
                        };
                        stats.reactive_guard_fragments += 1;
                        let guard_started = Instant::now();
                        let guard_name = format!(
                            "__reactive_guard_{}_{}",
                            binding.item.as_raw(),
                            clause_index
                        )
                        .into_boxed_str();
                        let guard_fragment = match if update.body_mode
                            == hir::ReactiveUpdateBodyMode::OptionalPayload
                        {
                            let signal_bool_type =
                                hir::GateType::Signal(Box::new(bool_type.clone()));
                            compile_runtime_expr_fragment(
                                self.module,
                                binding.item,
                                update.span,
                                update.guard,
                                &signal_bool_type,
                                guard_name.clone(),
                                &public_signals,
                                ReactiveFragmentRole::Guard,
                            )
                            .or_else(|_| {
                                compile_runtime_expr_fragment(
                                    self.module,
                                    binding.item,
                                    update.span,
                                    update.guard,
                                    &bool_type,
                                    guard_name,
                                    &public_signals,
                                    ReactiveFragmentRole::Guard,
                                )
                            })
                        } else {
                            compile_runtime_expr_fragment(
                                self.module,
                                binding.item,
                                update.span,
                                update.guard,
                                &bool_type,
                                guard_name,
                                &public_signals,
                                ReactiveFragmentRole::Guard,
                            )
                        } {
                            Ok(fragment) => {
                                stats.reactive_fragment_compile_duration += guard_started.elapsed();
                                fragment
                            }
                            Err(error) => {
                                stats.reactive_fragment_compile_duration += guard_started.elapsed();
                                errors.push(error);
                                continue;
                            }
                        };
                        stats.reactive_body_fragments += 1;
                        let body_started = Instant::now();
                        let body_name =
                            format!("__reactive_body_{}_{}", binding.item.as_raw(), clause_index)
                                .into_boxed_str();
                        let body_fragment = match if update.body_mode
                            == hir::ReactiveUpdateBodyMode::OptionalPayload
                        {
                            let signal_body_type =
                                hir::GateType::Signal(Box::new(body_type.clone()));
                            compile_runtime_expr_fragment(
                                self.module,
                                binding.item,
                                update.span,
                                update.body,
                                &signal_body_type,
                                body_name.clone(),
                                &public_signals,
                                ReactiveFragmentRole::Body,
                            )
                            .or_else(|_| {
                                compile_runtime_expr_fragment(
                                    self.module,
                                    binding.item,
                                    update.span,
                                    update.body,
                                    &body_type,
                                    body_name,
                                    &public_signals,
                                    ReactiveFragmentRole::Body,
                                )
                            })
                        } else {
                            compile_runtime_expr_fragment(
                                self.module,
                                binding.item,
                                update.span,
                                update.body,
                                &body_type,
                                body_name,
                                &public_signals,
                                ReactiveFragmentRole::Body,
                            )
                        } {
                            Ok(fragment) => {
                                stats.reactive_fragment_compile_duration += body_started.elapsed();
                                fragment
                            }
                            Err(error) => {
                                stats.reactive_fragment_compile_duration += body_started.elapsed();
                                errors.push(error);
                                continue;
                            }
                        };
                        let mut guard_dependencies = guard_fragment
                            .required_signals
                            .iter()
                            .map(|signal| signal.signal)
                            .collect::<Vec<_>>();
                        if guard_dependencies.is_empty() {
                            guard_dependencies = collect_direct_signal_dependencies(
                                self.module,
                                update.guard,
                                &public_signal_names,
                            );
                        }
                        let mut body_dependencies = signal_pipeline_dependencies.clone();
                        for signal in &body_fragment.required_signals {
                            push_unique_signal(&mut body_dependencies, signal.signal);
                        }
                        if body_dependencies.is_empty() {
                            body_dependencies = collect_direct_signal_dependencies(
                                self.module,
                                update.body,
                                &public_signal_names,
                            );
                        }
                        for dependency in guard_dependencies
                            .iter()
                            .chain(body_dependencies.iter())
                            .copied()
                        {
                            push_unique_signal(&mut reactive_dependencies, dependency);
                        }
                        if let Some(trigger_signal) = trigger_signal {
                            push_unique_signal(&mut reactive_dependencies, trigger_signal);
                        }
                        clause_bindings.push(HirReactiveUpdateBinding {
                            span: update.span,
                            keyword_span: update.keyword_span,
                            target_span: update.target_span,
                            guard: update.guard,
                            body: update.body,
                            body_mode: update.body_mode,
                            clause: ReactiveClauseHandle::from_raw(next_reactive_clause_raw),
                            trigger_signal,
                            guard_dependencies: guard_dependencies.clone().into_boxed_slice(),
                            body_dependencies: body_dependencies.clone().into_boxed_slice(),
                            compiled_guard: guard_fragment,
                            compiled_body: body_fragment,
                        });
                        next_reactive_clause_raw = next_reactive_clause_raw.wrapping_add(1);
                        clause_specs.push(
                            ReactiveClauseBuilderSpec::new(guard_dependencies, body_dependencies)
                                .with_trigger_signal(trigger_signal),
                        );
                    }
                    if let Some(source_input) = binding.source_input {
                        let source_signal = source_input.as_signal();
                        push_unique_signal(&mut reactive_dependencies, source_signal);
                        push_unique_signal(&mut resolved_seed_dependencies, source_signal);
                    }

                    if let Err(err) = graph_builder.define_reactive(
                        *reactive,
                        resolved_seed_dependencies.iter().copied(),
                        clause_specs,
                    ) {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                    *dependencies = reactive_dependencies.into_boxed_slice();
                    *seed_dependencies = resolved_seed_dependencies.into_boxed_slice();
                    *clauses = clause_bindings.into_boxed_slice();
                }
            }
        }

        let mut task_wakeups = BTreeMap::new();
        for node in self.recurrences.nodes() {
            if !self.includes_item(node.owner) {
                continue;
            }
            let hir::RecurrenceNodeOutcome::Planned(plan) = &node.outcome else {
                continue;
            };
            if plan.target.target() != RecurrenceTarget::Task {
                continue;
            }
            if task_wakeups.insert(node.owner, plan.wakeup).is_some() {
                errors.push(HirRuntimeAdapterError::DuplicateTaskOwner { owner: node.owner });
            }
        }

        let mut sources = Vec::new();
        let mut seen_source_owners = BTreeSet::new();
        for binding in &signals {
            let Some(hir::Item::Signal(signal)) = self.module.items().get(binding.item) else {
                continue;
            };
            if signal.source_metadata.is_none() {
                continue;
            }
            seen_source_owners.insert(binding.item);

            let Some(lifecycle_node) = report_index.source_lifecycles.get(&binding.item).copied()
            else {
                errors.push(HirRuntimeAdapterError::MissingSourceLifecycle {
                    owner: binding.item,
                });
                continue;
            };
            let lifecycle = match &lifecycle_node.outcome {
                hir::SourceLifecycleNodeOutcome::Planned(plan) => plan.clone(),
                hir::SourceLifecycleNodeOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedSourceLifecycle {
                        owner: binding.item,
                        source_span: lifecycle_node.source_span,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                    continue;
                }
            };

            let Some(decode_node) = report_index
                .source_decode_programs
                .get(&binding.item)
                .copied()
            else {
                errors.push(HirRuntimeAdapterError::MissingSourceDecodeProgram {
                    owner: binding.item,
                });
                continue;
            };
            let decode = match &decode_node.outcome {
                hir::SourceDecodeProgramOutcome::Planned(program) => program.clone(),
                hir::SourceDecodeProgramOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedSourceDecodeProgram {
                        owner: binding.item,
                        source_span: decode_node.source_span,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                    continue;
                }
            };

            let Some(input) = source_inputs.get(&binding.item).copied() else {
                errors.push(HirRuntimeAdapterError::MissingSourceInput {
                    owner: binding.item,
                });
                continue;
            };
            let spec = match self.build_source_spec(
                binding.item,
                input,
                &lifecycle,
                &decode,
                &public_signals,
            ) {
                Ok(spec) => spec,
                Err(mut source_errors) => {
                    errors.append(&mut source_errors);
                    continue;
                }
            };

            sources.push(HirSourceBinding {
                owner: binding.item,
                source_instance: lifecycle.instance,
                source_span: lifecycle_node.source_span,
                teardown: lifecycle.teardown,
                signal: binding.signal(),
                input,
                spec,
            });
        }

        for owner in report_index.source_lifecycles.keys() {
            if !self.includes_item(*owner) {
                continue;
            }
            if !seen_source_owners.contains(owner) {
                errors
                    .push(HirRuntimeAdapterError::UnexpectedSourceLifecycleOwner { owner: *owner });
            }
        }
        for owner in report_index.source_decode_programs.keys() {
            if !self.includes_item(*owner) {
                continue;
            }
            if !seen_source_owners.contains(owner) {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceDecodeOwner { owner: *owner });
            }
        }

        let mut tasks = Vec::new();
        for (item_id, item) in self.module.items().iter() {
            if !self.includes_item(item_id) {
                continue;
            }
            let hir::Item::Value(value) = item else {
                continue;
            };
            let Some(annotation) = value.annotation else {
                continue;
            };
            if !annotation_is_task(self.module, annotation) {
                continue;
            }

            let owner = match graph_builder.add_owner(value.name.text(), None) {
                Ok(owner) => owner,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            owners.push(HirOwnerBinding {
                item: item_id,
                span: value.header.span,
                name: value.name.text().into(),
                handle: owner,
            });

            let input =
                match graph_builder.add_input(format!("{}#task", value.name.text()), Some(owner)) {
                    Ok(input) => input,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
            let mut spec = TaskRuntimeSpec::new(runtime_task_instance(item_id), input);
            spec.wakeup = task_wakeups.get(&item_id).copied();
            tasks.push(HirTaskBinding {
                owner: item_id,
                owner_handle: owner,
                task_span: value.header.span,
                input,
                spec,
            });
        }

        let mut gates = Vec::new();
        let mut gate_sites = BTreeSet::new();
        for stage in self.gates.stages() {
            if !self.includes_item(stage.owner) {
                continue;
            }
            let site = HirGateStageId::new(stage.owner, stage.pipe_expr, stage.stage_index);
            if !gate_sites.insert(site) {
                errors.push(HirRuntimeAdapterError::DuplicateGateStage { site });
                continue;
            }
            let owner_signal = signal_derived_handle(&signals, stage.owner);
            match &stage.outcome {
                hir::GateStageOutcome::Ordinary(plan) => gates.push(HirGateStageBinding {
                    site,
                    stage_span: stage.stage_span,
                    predicate: stage.predicate,
                    owner_signal,
                    plan: HirRuntimeGatePlan::Ordinary(plan.clone()),
                }),
                hir::GateStageOutcome::SignalFilter(plan) => gates.push(HirGateStageBinding {
                    site,
                    stage_span: stage.stage_span,
                    predicate: stage.predicate,
                    owner_signal,
                    plan: HirRuntimeGatePlan::SignalFilter(plan.clone()),
                }),
                hir::GateStageOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedGateStage {
                        site,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                }
            }
        }

        let mut recurrences = Vec::new();
        let mut recurrence_sites = BTreeSet::new();
        for node in self.recurrences.nodes() {
            if !self.includes_item(node.owner) {
                continue;
            }
            let site = HirRecurrenceNodeId::new(node.owner, node.pipe_expr, node.start_stage_index);
            if !recurrence_sites.insert(site) {
                errors.push(HirRuntimeAdapterError::DuplicateRecurrenceNode { site });
                continue;
            }
            let owner_signal = signal_derived_handle(&signals, node.owner);
            match &node.outcome {
                hir::RecurrenceNodeOutcome::Planned(plan) => {
                    recurrences.push(HirRecurrenceBinding {
                        site,
                        start_stage_span: node.start_stage_span,
                        owner_signal,
                        wakeup_signal: plan.wakeup_signal,
                    });
                }
                hir::RecurrenceNodeOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedRecurrenceNode {
                        site,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                }
            }
        }

        let db_changed_bindings = collect_db_changed_bindings(self.module, self.included_items);

        if !errors.is_empty() {
            return Err(HirRuntimeAdapterErrors::new(errors));
        }

        let graph = graph_builder.build().map_err(|err| {
            HirRuntimeAdapterErrors::new(vec![HirRuntimeAdapterError::GraphBuild(err)])
        })?;
        let reactive_program =
            build_reactive_program(&graph, &signals, &sources, &tasks, &recurrences);

        Ok(ProfiledHirRuntimeAssembly {
            assembly: HirRuntimeAssembly {
                graph,
                reactive_program,
                owners: owners.into_boxed_slice(),
                signals: signals.into_boxed_slice(),
                sources: sources.into_boxed_slice(),
                tasks: tasks.into_boxed_slice(),
                gates: gates.into_boxed_slice(),
                recurrences: recurrences.into_boxed_slice(),
                db_changed_bindings,
            },
            stats,
        })
    }

    fn build_source_spec(
        &self,
        owner: hir::ItemId,
        input: InputHandle,
        lifecycle: &hir::SourceLifecyclePlan,
        decode: &hir::SourceDecodeProgram,
        public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    ) -> Result<SourceRuntimeSpec<hir::SourceDecodeProgram>, Vec<HirRuntimeAdapterError>> {
        let mut errors = Vec::new();
        let provider = match adapt_source_provider(owner, &lifecycle.provider) {
            Ok(provider) => provider,
            Err(error) => {
                errors.push(error);
                RuntimeSourceProvider::custom("<invalid-source-provider>")
            }
        };
        let reconfiguration_dependencies = resolve_signal_dependencies(
            self.module,
            owner,
            &lifecycle.reconfiguration_dependencies,
            public_signals,
            &mut errors,
        );
        let explicit_triggers = lifecycle
            .explicit_triggers
            .iter()
            .filter_map(|binding| {
                resolve_source_option_binding(
                    self.module,
                    owner,
                    binding,
                    public_signals,
                    &mut errors,
                )
            })
            .collect::<Vec<_>>();
        let active_when = lifecycle.active_when.as_ref().and_then(|binding| {
            resolve_source_option_binding(self.module, owner, binding, public_signals, &mut errors)
        });

        if !errors.is_empty() {
            return Err(errors);
        }

        let mut spec =
            SourceRuntimeSpec::new(runtime_source_instance(lifecycle.instance), input, provider);
        spec.reconfiguration_dependencies = reconfiguration_dependencies.into_boxed_slice();
        spec.explicit_triggers = explicit_triggers.into_boxed_slice();
        spec.active_when = active_when;
        spec.cancellation = lifecycle.cancellation;
        spec.replacement = match lifecycle.replacement {
            hir::SourceReplacementPolicy::DisposeSupersededBeforePublish => {
                SourceReplacementPolicy::DisposeSupersededBeforePublish
            }
        };
        spec.stale_work = match lifecycle.stale_work {
            hir::SourceStaleWorkPolicy::DropStalePublications => {
                SourceStaleWorkPolicy::DropStalePublications
            }
        };
        spec.decode = Some(decode.clone());
        Ok(spec)
    }
}

fn collect_db_changed_bindings(
    module: &hir::Module,
    included_items: Option<&HashSet<hir::ItemId>>,
) -> Box<[HirDbChangedBinding]> {
    let mut bindings = BTreeSet::new();
    for (expr_id, expr) in module.exprs().iter() {
        let hir::ExprKind::Projection {
            base: hir::ProjectionBase::Expr(base),
            path,
        } = &expr.kind
        else {
            continue;
        };
        if path.segments().len() != 1 || path.segments().first().text() != "changed" {
            continue;
        }
        let Some(table_item) = resolve_db_changed_projection_base_item(module, *base) else {
            continue;
        };
        let changed_signals = hir::collect_signal_dependencies_for_expr(module, expr_id);
        let [changed_signal] = changed_signals.as_slice() else {
            continue;
        };
        if !included_items.is_none_or(|items| items.contains(&table_item))
            || !included_items.is_none_or(|items| items.contains(changed_signal))
        {
            continue;
        }
        bindings.insert(HirDbChangedBinding {
            table_item,
            changed_signal: *changed_signal,
        });
    }
    bindings.into_iter().collect::<Vec<_>>().into_boxed_slice()
}

fn resolve_db_changed_projection_base_item(
    module: &hir::Module,
    expr: hir::ExprId,
) -> Option<hir::ItemId> {
    let hir::ExprKind::Name(reference) = &module.exprs()[expr].kind else {
        return None;
    };
    let hir::ResolutionState::Resolved(hir::TermResolution::Item(item)) = reference.resolution
    else {
        return None;
    };
    Some(item)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRuntimeAssembly {
    graph: SignalGraph,
    reactive_program: ReactiveProgram,
    owners: Box<[HirOwnerBinding]>,
    signals: Box<[HirSignalBinding]>,
    sources: Box<[HirSourceBinding]>,
    tasks: Box<[HirTaskBinding]>,
    gates: Box<[HirGateStageBinding]>,
    recurrences: Box<[HirRecurrenceBinding]>,
    db_changed_bindings: Box<[HirDbChangedBinding]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRuntimeAssemblyParts {
    pub graph: crate::SignalGraphParts,
    pub reactive_program: crate::ReactiveProgramParts,
    pub owners: Box<[HirOwnerBinding]>,
    pub signals: Box<[HirSignalBinding]>,
    pub sources: Box<[HirSourceBinding]>,
    pub tasks: Box<[HirTaskBinding]>,
    pub gates: Box<[HirGateStageBinding]>,
    pub recurrences: Box<[HirRecurrenceBinding]>,
    pub db_changed_bindings: Box<[HirDbChangedBinding]>,
}

impl HirRuntimeAssembly {
    pub fn graph(&self) -> &SignalGraph {
        &self.graph
    }

    pub fn reactive_program(&self) -> &ReactiveProgram {
        &self.reactive_program
    }

    pub fn owners(&self) -> &[HirOwnerBinding] {
        &self.owners
    }

    pub fn signals(&self) -> &[HirSignalBinding] {
        &self.signals
    }

    pub fn sources(&self) -> &[HirSourceBinding] {
        &self.sources
    }

    pub fn tasks(&self) -> &[HirTaskBinding] {
        &self.tasks
    }

    pub fn gates(&self) -> &[HirGateStageBinding] {
        &self.gates
    }

    pub fn recurrences(&self) -> &[HirRecurrenceBinding] {
        &self.recurrences
    }

    pub fn db_changed_bindings(&self) -> &[HirDbChangedBinding] {
        &self.db_changed_bindings
    }

    pub fn owner(&self, item: hir::ItemId) -> Option<&HirOwnerBinding> {
        self.owners.iter().find(|binding| binding.item == item)
    }

    pub fn signal(&self, item: hir::ItemId) -> Option<&HirSignalBinding> {
        self.signals.iter().find(|binding| binding.item == item)
    }

    pub fn source_by_owner(&self, item: hir::ItemId) -> Option<&HirSourceBinding> {
        self.sources.iter().find(|binding| binding.owner == item)
    }

    pub fn task_by_owner(&self, item: hir::ItemId) -> Option<&HirTaskBinding> {
        self.tasks.iter().find(|binding| binding.owner == item)
    }

    pub fn instantiate_runtime<V>(
        &self,
    ) -> Result<TaskSourceRuntime<V, hir::SourceDecodeProgram>, HirRuntimeInstantiationError> {
        self.instantiate_runtime_with_value_store(InlineCommittedValueStore::default())
    }

    pub fn instantiate_runtime_with_value_store<V, S>(
        &self,
        storage: S,
    ) -> Result<TaskSourceRuntime<V, hir::SourceDecodeProgram, S>, HirRuntimeInstantiationError>
    where
        S: CommittedValueStore<V>,
    {
        let mut runtime = TaskSourceRuntime::with_value_store(self.graph.clone(), storage);
        for source in &self.sources {
            runtime
                .register_source(source.spec.clone())
                .map_err(|error| HirRuntimeInstantiationError::RegisterSource {
                    owner: source.owner,
                    instance: source.spec.instance,
                    error,
                })?;
        }
        for task in &self.tasks {
            runtime.register_task(task.spec.clone()).map_err(|error| {
                HirRuntimeInstantiationError::RegisterTask {
                    owner: task.owner,
                    instance: task.spec.instance,
                    error,
                }
            })?;
        }
        Ok(runtime)
    }

    pub fn into_parts(self) -> HirRuntimeAssemblyParts {
        let Self {
            graph,
            reactive_program,
            owners,
            signals,
            sources,
            tasks,
            gates,
            recurrences,
            db_changed_bindings,
        } = self;
        HirRuntimeAssemblyParts {
            graph: graph.into_parts(),
            reactive_program: reactive_program.into_parts(),
            owners,
            signals,
            sources,
            tasks,
            gates,
            recurrences,
            db_changed_bindings,
        }
    }

    pub fn from_parts(parts: HirRuntimeAssemblyParts) -> Self {
        let HirRuntimeAssemblyParts {
            graph,
            reactive_program,
            owners,
            signals,
            sources,
            tasks,
            gates,
            recurrences,
            db_changed_bindings,
        } = parts;
        Self {
            graph: SignalGraph::from_parts(graph),
            reactive_program: ReactiveProgram::from_parts(reactive_program),
            owners,
            signals,
            sources,
            tasks,
            gates,
            recurrences,
            db_changed_bindings,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct HirDbChangedBinding {
    pub table_item: hir::ItemId,
    pub changed_signal: hir::ItemId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirOwnerBinding {
    pub item: hir::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub handle: OwnerHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirSignalBinding {
    pub item: hir::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub owner: OwnerHandle,
    pub kind: HirSignalBindingKind,
    pub source_input: Option<InputHandle>,
}

impl HirSignalBinding {
    pub fn signal(&self) -> SignalHandle {
        match self.kind {
            HirSignalBindingKind::Input { signal } => signal.as_signal(),
            HirSignalBindingKind::Derived { signal, .. } => signal.as_signal(),
            HirSignalBindingKind::Reactive { signal, .. } => signal,
        }
    }

    pub fn input(&self) -> Option<InputHandle> {
        match self.kind {
            HirSignalBindingKind::Input { signal } => Some(signal),
            HirSignalBindingKind::Derived { .. } | HirSignalBindingKind::Reactive { .. } => None,
        }
    }

    pub fn derived(&self) -> Option<DerivedHandle> {
        match self.kind {
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Reactive { .. } => None,
            HirSignalBindingKind::Derived { signal, .. } => Some(signal),
        }
    }

    pub fn dependencies(&self) -> &[SignalHandle] {
        match &self.kind {
            HirSignalBindingKind::Input { .. } => &[],
            HirSignalBindingKind::Derived { dependencies, .. } => dependencies,
            HirSignalBindingKind::Reactive { dependencies, .. } => dependencies,
        }
    }

    pub fn temporal_helper_inputs(&self) -> &[InputHandle] {
        match &self.kind {
            HirSignalBindingKind::Derived {
                temporal_helpers, ..
            } => temporal_helpers,
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Reactive { .. } => &[],
        }
    }

    pub fn temporal_trigger_dependencies(&self) -> &[SignalHandle] {
        match &self.kind {
            HirSignalBindingKind::Derived {
                temporal_trigger_dependencies,
                ..
            } => temporal_trigger_dependencies,
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Reactive { .. } => &[],
        }
    }

    pub fn reactive_updates(&self) -> &[HirReactiveUpdateBinding] {
        match &self.kind {
            HirSignalBindingKind::Reactive { clauses, .. } => clauses,
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Derived { .. } => &[],
        }
    }

    pub fn reactive_seed_dependencies(&self) -> &[SignalHandle] {
        match &self.kind {
            HirSignalBindingKind::Reactive {
                seed_dependencies, ..
            } => seed_dependencies,
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Derived { .. } => &[],
        }
    }

    pub fn reactive_signal(&self) -> Option<SignalHandle> {
        match self.kind {
            HirSignalBindingKind::Reactive { signal, .. } => Some(signal),
            HirSignalBindingKind::Input { .. } | HirSignalBindingKind::Derived { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirSignalBindingKind {
    Input {
        signal: InputHandle,
    },
    Derived {
        signal: DerivedHandle,
        dependencies: Box<[SignalHandle]>,
        temporal_trigger_dependencies: Box<[SignalHandle]>,
        temporal_helpers: Box<[InputHandle]>,
    },
    Reactive {
        signal: SignalHandle,
        dependencies: Box<[SignalHandle]>,
        seed_dependencies: Box<[SignalHandle]>,
        clauses: Box<[HirReactiveUpdateBinding]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirReactiveUpdateBinding {
    pub span: SourceSpan,
    pub keyword_span: SourceSpan,
    pub target_span: SourceSpan,
    pub guard: hir::ExprId,
    pub body: hir::ExprId,
    pub body_mode: hir::ReactiveUpdateBodyMode,
    pub clause: ReactiveClauseHandle,
    pub trigger_signal: Option<SignalHandle>,
    pub guard_dependencies: Box<[SignalHandle]>,
    pub body_dependencies: Box<[SignalHandle]>,
    pub compiled_guard: HirCompiledRuntimeExpr,
    pub compiled_body: HirCompiledRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirCompiledRuntimeExpr {
    pub backend: Arc<BackendProgram>,
    pub native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    pub entry_item: BackendItemId,
    pub required_signals: Box<[HirCompiledRuntimeExprSignal]>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirCompiledRuntimeExprSignal {
    pub signal: SignalHandle,
    pub backend_item: BackendItemId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirSourceBinding {
    pub owner: hir::ItemId,
    pub source_instance: hir::SourceInstanceId,
    pub source_span: SourceSpan,
    pub teardown: hir::SourceTeardownPolicy,
    pub signal: SignalHandle,
    pub input: InputHandle,
    pub spec: SourceRuntimeSpec<hir::SourceDecodeProgram>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirTaskBinding {
    pub owner: hir::ItemId,
    pub owner_handle: OwnerHandle,
    pub task_span: SourceSpan,
    pub input: InputHandle,
    pub spec: TaskRuntimeSpec,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct HirGateStageId {
    pub owner: hir::ItemId,
    pub pipe_expr: hir::ExprId,
    pub stage_index: usize,
}

impl HirGateStageId {
    pub const fn new(owner: hir::ItemId, pipe_expr: hir::ExprId, stage_index: usize) -> Self {
        Self {
            owner,
            pipe_expr,
            stage_index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HirRuntimeGatePlan {
    Ordinary(hir::OrdinaryGateStage),
    SignalFilter(hir::SignalGateFilter),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirGateStageBinding {
    pub site: HirGateStageId,
    pub stage_span: SourceSpan,
    pub predicate: hir::ExprId,
    pub owner_signal: Option<DerivedHandle>,
    pub plan: HirRuntimeGatePlan,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct HirRecurrenceNodeId {
    pub owner: hir::ItemId,
    pub pipe_expr: hir::ExprId,
    pub start_stage_index: usize,
}

impl HirRecurrenceNodeId {
    pub const fn new(owner: hir::ItemId, pipe_expr: hir::ExprId, start_stage_index: usize) -> Self {
        Self {
            owner,
            pipe_expr,
            start_stage_index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HirRecurrenceBinding {
    pub site: HirRecurrenceNodeId,
    pub start_stage_span: SourceSpan,
    pub owner_signal: Option<DerivedHandle>,
    pub wakeup_signal: Option<hir::ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirRuntimeInstantiationError {
    RegisterSource {
        owner: hir::ItemId,
        instance: SourceInstanceId,
        error: TaskSourceRuntimeError,
    },
    RegisterTask {
        owner: hir::ItemId,
        instance: TaskInstanceId,
        error: TaskSourceRuntimeError,
    },
}

impl fmt::Display for HirRuntimeInstantiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegisterSource {
                owner,
                instance,
                error,
            } => write!(
                f,
                "failed to register adapted source {instance:?} for owner {owner}: {error:?}"
            ),
            Self::RegisterTask {
                owner,
                instance,
                error,
            } => write!(
                f,
                "failed to register adapted task {instance:?} for owner {owner}: {error:?}"
            ),
        }
    }
}

impl std::error::Error for HirRuntimeInstantiationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRuntimeAdapterErrors {
    errors: Box<[HirRuntimeAdapterError]>,
}

impl HirRuntimeAdapterErrors {
    pub fn new(errors: Vec<HirRuntimeAdapterError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[HirRuntimeAdapterError] {
        &self.errors
    }
}

impl fmt::Display for HirRuntimeAdapterErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "failed to adapt HIR into a runtime assembly:")?;
        for error in &self.errors {
            writeln!(f, "- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for HirRuntimeAdapterErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirRuntimeAdapterError {
    SignalHasNoRuntimeNode {
        item: hir::ItemId,
        span: SourceSpan,
    },
    MissingSourceInput {
        owner: hir::ItemId,
    },
    MissingSourceLifecycle {
        owner: hir::ItemId,
    },
    MissingSourceDecodeProgram {
        owner: hir::ItemId,
    },
    UnexpectedSourceLifecycleOwner {
        owner: hir::ItemId,
    },
    UnexpectedSourceDecodeOwner {
        owner: hir::ItemId,
    },
    DuplicateTaskOwner {
        owner: hir::ItemId,
    },
    BlockedSourceLifecycle {
        owner: hir::ItemId,
        source_span: SourceSpan,
        blockers: Box<[hir::SourceLifecycleElaborationBlocker]>,
    },
    BlockedSourceDecodeProgram {
        owner: hir::ItemId,
        source_span: SourceSpan,
        blockers: Box<[hir::SourceDecodeProgramBlocker]>,
    },
    MissingSourceProvider {
        owner: hir::ItemId,
    },
    InvalidSourceProviderShape {
        owner: hir::ItemId,
        key: Box<str>,
    },
    UnknownSignalDependency {
        owner: hir::ItemId,
        dependency: hir::ItemId,
    },
    DependencyIsNotSignal {
        owner: hir::ItemId,
        dependency: hir::ItemId,
        kind: hir::ItemKind,
    },
    UnsupportedSourceOptionSignalExpr {
        owner: hir::ItemId,
        option_name: Box<str>,
        expr: hir::ExprId,
    },
    SourceOptionBindingNotSignal {
        owner: hir::ItemId,
        option_name: Box<str>,
        item: hir::ItemId,
        kind: hir::ItemKind,
    },
    DuplicateGateStage {
        site: HirGateStageId,
    },
    BlockedGateStage {
        site: HirGateStageId,
        blockers: Box<[hir::GateElaborationBlocker]>,
    },
    DuplicateRecurrenceNode {
        site: HirRecurrenceNodeId,
    },
    BlockedRecurrenceNode {
        site: HirRecurrenceNodeId,
        blockers: Box<[hir::RecurrenceElaborationBlocker]>,
    },
    ReactiveUpdateUnknownPayloadType {
        owner: hir::ItemId,
        clause_span: SourceSpan,
    },
    BlockedReactiveUpdateGuard {
        owner: hir::ItemId,
        clause_span: SourceSpan,
        blockers: Box<[hir::GeneralExprBlocker]>,
    },
    BlockedReactiveUpdateBody {
        owner: hir::ItemId,
        clause_span: SourceSpan,
        blockers: Box<[hir::GeneralExprBlocker]>,
    },
    ReactiveUpdateFragmentLowering {
        owner: hir::ItemId,
        clause_span: SourceSpan,
        role: &'static str,
        stage: &'static str,
        message: Box<str>,
    },
    ReactiveUpdateFragmentMissingEntry {
        owner: hir::ItemId,
        clause_span: SourceSpan,
        role: &'static str,
        entry_name: Box<str>,
    },
    ReactiveUpdateFragmentUnknownSignal {
        owner: hir::ItemId,
        clause_span: SourceSpan,
        role: &'static str,
        dependency: hir::ItemId,
    },
    GraphBuild(GraphBuildError),
}

impl fmt::Display for HirRuntimeAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignalHasNoRuntimeNode { item, .. } => write!(
                f,
                "signal item {item} has neither a body nor an @source input to adapt"
            ),
            Self::MissingSourceInput { owner } => {
                write!(
                    f,
                    "source-backed signal {owner} is missing its source input handle"
                )
            }
            Self::MissingSourceLifecycle { owner } => write!(
                f,
                "source-backed signal {owner} has no source lifecycle handoff"
            ),
            Self::MissingSourceDecodeProgram { owner } => write!(
                f,
                "source-backed signal {owner} has no source decode program handoff"
            ),
            Self::UnexpectedSourceLifecycleOwner { owner } => write!(
                f,
                "source lifecycle report references non-source owner {owner}"
            ),
            Self::UnexpectedSourceDecodeOwner { owner } => write!(
                f,
                "source decode report references non-source owner {owner}"
            ),
            Self::DuplicateTaskOwner { owner } => {
                write!(
                    f,
                    "task owner {owner} exposes more than one recurrence handoff"
                )
            }
            Self::BlockedSourceLifecycle {
                owner, blockers, ..
            } => write!(
                f,
                "source lifecycle for owner {owner} is blocked: {blockers:?}"
            ),
            Self::BlockedSourceDecodeProgram {
                owner, blockers, ..
            } => write!(
                f,
                "source decode program for owner {owner} is blocked: {blockers:?}"
            ),
            Self::MissingSourceProvider { owner } => write!(
                f,
                "source owner {owner} has no provider: the @source decorator is missing a provider path"
            ),
            Self::InvalidSourceProviderShape { owner, key } => write!(
                f,
                "source owner {owner} has an invalid provider path shape: {key:?} is not a valid provider identifier"
            ),
            Self::UnknownSignalDependency { owner, dependency } => write!(
                f,
                "signal owner {owner} depends on unknown or unbound signal item {dependency}"
            ),
            Self::DependencyIsNotSignal {
                owner,
                dependency,
                kind,
            } => write!(
                f,
                "signal owner {owner} depends on item {dependency}, but it is a {kind:?}"
            ),
            Self::UnsupportedSourceOptionSignalExpr {
                owner,
                option_name,
                expr,
            } => write!(
                f,
                "source owner {owner} uses `{option_name}` expression {expr} that is not a direct signal reference"
            ),
            Self::SourceOptionBindingNotSignal {
                owner,
                option_name,
                item,
                kind,
            } => write!(
                f,
                "source owner {owner} binds `{option_name}` to item {item}, but it is a {kind:?}, not a signal"
            ),
            Self::DuplicateGateStage { site } => {
                write!(f, "duplicate gate handoff encountered at {site:?}")
            }
            Self::BlockedGateStage { site, blockers } => {
                write!(f, "gate handoff {site:?} is blocked: {blockers:?}")
            }
            Self::DuplicateRecurrenceNode { site } => {
                write!(f, "duplicate recurrence handoff encountered at {site:?}")
            }
            Self::BlockedRecurrenceNode { site, blockers } => {
                write!(f, "recurrence handoff {site:?} is blocked: {blockers:?}")
            }
            Self::ReactiveUpdateUnknownPayloadType { owner, clause_span } => write!(
                f,
                "reactive update for owner {owner} at {clause_span:?} has no known signal payload type"
            ),
            Self::BlockedReactiveUpdateGuard {
                owner,
                clause_span,
                blockers,
            } => write!(
                f,
                "reactive update guard for owner {owner} at {clause_span:?} is blocked: {blockers:?}"
            ),
            Self::BlockedReactiveUpdateBody {
                owner,
                clause_span,
                blockers,
            } => write!(
                f,
                "reactive update body for owner {owner} at {clause_span:?} is blocked: {blockers:?}"
            ),
            Self::ReactiveUpdateFragmentLowering {
                owner,
                clause_span,
                role,
                stage,
                message,
            } => write!(
                f,
                "failed to lower reactive update {role} fragment for owner {owner} at {clause_span:?} during {stage}: {message}"
            ),
            Self::ReactiveUpdateFragmentMissingEntry {
                owner,
                clause_span,
                role,
                entry_name,
            } => write!(
                f,
                "reactive update {role} fragment for owner {owner} at {clause_span:?} is missing entry item `{entry_name}`"
            ),
            Self::ReactiveUpdateFragmentUnknownSignal {
                owner,
                clause_span,
                role,
                dependency,
            } => write!(
                f,
                "reactive update {role} fragment for owner {owner} at {clause_span:?} depends on signal item {dependency} with no runtime binding"
            ),
            Self::GraphBuild(error) => write!(f, "signal graph build failed: {error:?}"),
        }
    }
}

impl std::error::Error for HirRuntimeAdapterError {}

#[derive(Clone, Debug)]
struct ReportIndex<'a> {
    source_lifecycles: BTreeMap<hir::ItemId, &'a hir::SourceLifecycleNodeElaboration>,
    source_decode_programs: BTreeMap<hir::ItemId, &'a hir::SourceDecodeProgramNode>,
}

impl<'a> ReportIndex<'a> {
    fn new(
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        errors: &mut Vec<HirRuntimeAdapterError>,
    ) -> Self {
        let mut lifecycle_map = BTreeMap::new();
        for node in source_lifecycles.nodes() {
            if lifecycle_map.insert(node.owner, node).is_some() {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceLifecycleOwner {
                    owner: node.owner,
                });
            }
        }

        let mut decode_map = BTreeMap::new();
        for node in source_decode_programs.nodes() {
            if decode_map.insert(node.owner, node).is_some() {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceDecodeOwner {
                    owner: node.owner,
                });
            }
        }

        Self {
            source_lifecycles: lifecycle_map,
            source_decode_programs: decode_map,
        }
    }
}

fn signal_derived_handle(
    signals: &[HirSignalBinding],
    owner: hir::ItemId,
) -> Option<DerivedHandle> {
    signals
        .iter()
        .find(|binding| binding.item == owner)
        .and_then(HirSignalBinding::derived)
}

fn runtime_source_instance(instance: hir::SourceInstanceId) -> SourceInstanceId {
    SourceInstanceId::from_raw(instance.decorator().as_raw())
}

fn runtime_task_instance(item: hir::ItemId) -> TaskInstanceId {
    TaskInstanceId::from_raw(item.as_raw())
}

fn annotation_is_task(module: &hir::Module, ty: hir::TypeId) -> bool {
    type_head_with_arity(module, ty).is_some_and(|(head, arity)| {
        head == hir::TypeResolution::Builtin(hir::BuiltinType::Task) && arity == 2
    })
}

fn type_head_with_arity(
    module: &hir::Module,
    mut ty: hir::TypeId,
) -> Option<(hir::TypeResolution, usize)> {
    let mut arity = 0usize;
    loop {
        match &module.types()[ty].kind {
            hir::TypeKind::Apply { callee, arguments } => {
                arity += arguments.len();
                ty = *callee;
            }
            hir::TypeKind::Name(reference) => {
                return match reference.resolution {
                    hir::ResolutionState::Resolved(head) => Some((head, arity)),
                    hir::ResolutionState::Unresolved => None,
                };
            }
            hir::TypeKind::Tuple(_)
            | hir::TypeKind::Record(_)
            | hir::TypeKind::RecordTransform { .. }
            | hir::TypeKind::Arrow { .. } => {
                return None;
            }
        }
    }
}

fn queue_patch_block_exprs(patch: &hir::PatchBlock, work: &mut VecDeque<hir::ExprId>) {
    for entry in &patch.entries {
        for segment in &entry.selector.segments {
            if let hir::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                work.push_back(*expr);
            }
        }
        match &entry.instruction.kind {
            hir::PatchInstructionKind::Replace(expr) | hir::PatchInstructionKind::Store(expr) => {
                work.push_back(*expr);
            }
            hir::PatchInstructionKind::Remove => {}
        }
    }
}

fn adapt_source_provider(
    owner: hir::ItemId,
    provider: &hir::SourceProviderRef,
) -> Result<RuntimeSourceProvider, HirRuntimeAdapterError> {
    match provider {
        hir::SourceProviderRef::Builtin(provider) => Ok(RuntimeSourceProvider::builtin(*provider)),
        hir::SourceProviderRef::Custom(key) => Ok(RuntimeSourceProvider::custom(key.clone())),
        hir::SourceProviderRef::Missing => {
            Err(HirRuntimeAdapterError::MissingSourceProvider { owner })
        }
        hir::SourceProviderRef::InvalidShape(key) => {
            Err(HirRuntimeAdapterError::InvalidSourceProviderShape {
                owner,
                key: key.clone(),
            })
        }
    }
}

fn resolve_signal_dependencies(
    module: &hir::Module,
    owner: hir::ItemId,
    dependencies: &[hir::ItemId],
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    errors: &mut Vec<HirRuntimeAdapterError>,
) -> Vec<SignalHandle> {
    let mut resolved = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        match module.items().get(*dependency) {
            Some(hir::Item::Signal(_)) => match public_signals.get(dependency).copied() {
                Some(handle) => resolved.push(handle),
                None => errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                    owner,
                    dependency: *dependency,
                }),
            },
            Some(item) => errors.push(HirRuntimeAdapterError::DependencyIsNotSignal {
                owner,
                dependency: *dependency,
                kind: item.kind(),
            }),
            None => errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                owner,
                dependency: *dependency,
            }),
        }
    }
    resolved
}

fn push_unique_signal(signals: &mut Vec<SignalHandle>, signal: SignalHandle) {
    if !signals.contains(&signal) {
        signals.push(signal);
    }
}

fn collect_direct_signal_dependencies(
    module: &hir::Module,
    expr: hir::ExprId,
    public_signal_names: &BTreeMap<String, SignalHandle>,
) -> Vec<SignalHandle> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    let mut work = VecDeque::from([expr]);

    while let Some(expr) = work.pop_front() {
        if !seen.insert(expr) {
            continue;
        }
        match &module.exprs()[expr].kind {
            hir::ExprKind::Name(reference) => {
                if let Some(signal) = public_signal_names
                    .get(&reference.path.to_string())
                    .copied()
                {
                    push_unique_signal(&mut resolved, signal);
                }
            }
            hir::ExprKind::Text(text) => {
                for segment in &text.segments {
                    if let hir::TextSegment::Interpolation(interpolation) = segment {
                        work.push_back(interpolation.expr);
                    }
                }
            }
            hir::ExprKind::Tuple(elements) => {
                work.extend(elements.iter().copied());
            }
            hir::ExprKind::List(elements) | hir::ExprKind::Set(elements) => {
                work.extend(elements.iter().copied());
            }
            hir::ExprKind::Map(map) => {
                for entry in &map.entries {
                    work.push_back(entry.key);
                    work.push_back(entry.value);
                }
            }
            hir::ExprKind::Lambda(lambda) => {
                work.push_back(lambda.body);
            }
            hir::ExprKind::Record(record) => {
                work.extend(record.fields.iter().map(|field| field.value));
            }
            hir::ExprKind::Projection { base, .. } => {
                if let hir::ProjectionBase::Expr(base) = base {
                    work.push_back(*base);
                }
            }
            hir::ExprKind::Apply { callee, arguments } => {
                work.push_back(*callee);
                work.extend(arguments.iter().copied());
            }
            hir::ExprKind::Unary { expr, .. } => work.push_back(*expr),
            hir::ExprKind::Binary { left, right, .. } => {
                work.push_back(*left);
                work.push_back(*right);
            }
            hir::ExprKind::PatchApply { target, patch } => {
                work.push_back(*target);
                queue_patch_block_exprs(patch, &mut work);
            }
            hir::ExprKind::PatchLiteral(patch) => {
                queue_patch_block_exprs(patch, &mut work);
            }
            hir::ExprKind::Pipe(pipe) => {
                work.push_back(pipe.head);
                for stage in pipe.stages.iter() {
                    match &stage.kind {
                        hir::PipeStageKind::Transform { expr }
                        | hir::PipeStageKind::Gate { expr }
                        | hir::PipeStageKind::Map { expr }
                        | hir::PipeStageKind::Apply { expr }
                        | hir::PipeStageKind::Tap { expr }
                        | hir::PipeStageKind::FanIn { expr }
                        | hir::PipeStageKind::Truthy { expr }
                        | hir::PipeStageKind::Falsy { expr }
                        | hir::PipeStageKind::RecurStart { expr }
                        | hir::PipeStageKind::RecurStep { expr }
                        | hir::PipeStageKind::Validate { expr }
                        | hir::PipeStageKind::Previous { expr }
                        | hir::PipeStageKind::Diff { expr } => work.push_back(*expr),
                        hir::PipeStageKind::Delay { duration } => work.push_back(*duration),
                        hir::PipeStageKind::Burst { every, count } => {
                            work.push_back(*every);
                            work.push_back(*count);
                        }
                        hir::PipeStageKind::Accumulate { seed, step } => {
                            work.push_back(*seed);
                            work.push_back(*step);
                        }
                        hir::PipeStageKind::Case { body, .. } => {
                            work.push_back(*body);
                        }
                    }
                }
            }
            hir::ExprKind::Integer(_)
            | hir::ExprKind::Float(_)
            | hir::ExprKind::Decimal(_)
            | hir::ExprKind::BigInt(_)
            | hir::ExprKind::SuffixedInteger(_)
            | hir::ExprKind::AmbientSubject
            | hir::ExprKind::Regex(_)
            | hir::ExprKind::Cluster(_)
            | hir::ExprKind::Markup(_) => {}
        }
    }

    resolved
}

fn collect_pipe_stage_signal_dependencies(
    module: &hir::Module,
    expr: hir::ExprId,
    public_signal_names: &BTreeMap<String, SignalHandle>,
) -> Vec<SignalHandle> {
    let hir::ExprKind::Pipe(pipe) = &module.exprs()[expr].kind else {
        return Vec::new();
    };
    let mut resolved = Vec::new();
    for stage in pipe.stages.iter() {
        let exprs = match &stage.kind {
            hir::PipeStageKind::Transform { expr }
            | hir::PipeStageKind::Gate { expr }
            | hir::PipeStageKind::Map { expr }
            | hir::PipeStageKind::Apply { expr }
            | hir::PipeStageKind::Tap { expr }
            | hir::PipeStageKind::FanIn { expr }
            | hir::PipeStageKind::Truthy { expr }
            | hir::PipeStageKind::Falsy { expr }
            | hir::PipeStageKind::RecurStart { expr }
            | hir::PipeStageKind::RecurStep { expr }
            | hir::PipeStageKind::Validate { expr }
            | hir::PipeStageKind::Previous { expr }
            | hir::PipeStageKind::Diff { expr }
            | hir::PipeStageKind::Delay { duration: expr } => vec![*expr],
            hir::PipeStageKind::Burst { every, count } => vec![*every, *count],
            hir::PipeStageKind::Accumulate { seed, step } => vec![*seed, *step],
            hir::PipeStageKind::Case { body, .. } => vec![*body],
        };
        for stage_expr in exprs {
            for signal in
                collect_direct_signal_dependencies(module, stage_expr, public_signal_names)
            {
                push_unique_signal(&mut resolved, signal);
            }
        }
    }
    resolved
}

fn add_temporal_helper_inputs(
    module: &hir::Module,
    graph_builder: &mut SignalGraphBuilder,
    signal_name: &str,
    owner: OwnerHandle,
    expr: hir::ExprId,
) -> Result<Box<[InputHandle]>, GraphBuildError> {
    let helper_count = collect_temporal_helper_count(module, expr);
    let mut helpers = Vec::with_capacity(helper_count);
    for index in 0..helper_count {
        let input =
            graph_builder.add_input(format!("{signal_name}#temporal{}", index + 1), Some(owner))?;
        helpers.push(input);
    }
    Ok(helpers.into_boxed_slice())
}

fn collect_temporal_helper_count(module: &hir::Module, expr: hir::ExprId) -> usize {
    let mut total = 0usize;
    let mut work = vec![expr];
    while let Some(expr_id) = work.pop() {
        let expr = &module.exprs()[expr_id];
        match &expr.kind {
            hir::ExprKind::Pipe(pipe) => {
                for stage in pipe.stages.iter().rev() {
                    match &stage.kind {
                        hir::PipeStageKind::Delay { .. } | hir::PipeStageKind::Burst { .. } => {
                            total += 1;
                        }
                        _ => {}
                    }
                    match &stage.kind {
                        hir::PipeStageKind::Transform { expr }
                        | hir::PipeStageKind::Gate { expr }
                        | hir::PipeStageKind::Map { expr }
                        | hir::PipeStageKind::Apply { expr }
                        | hir::PipeStageKind::Tap { expr }
                        | hir::PipeStageKind::FanIn { expr }
                        | hir::PipeStageKind::Truthy { expr }
                        | hir::PipeStageKind::Falsy { expr }
                        | hir::PipeStageKind::RecurStart { expr }
                        | hir::PipeStageKind::RecurStep { expr }
                        | hir::PipeStageKind::Validate { expr }
                        | hir::PipeStageKind::Previous { expr }
                        | hir::PipeStageKind::Diff { expr }
                        | hir::PipeStageKind::Delay { duration: expr } => work.push(*expr),
                        hir::PipeStageKind::Burst { every, count } => {
                            work.push(*count);
                            work.push(*every);
                        }
                        hir::PipeStageKind::Accumulate { seed, step } => {
                            work.push(*step);
                            work.push(*seed);
                        }
                        hir::PipeStageKind::Case { body, .. } => {
                            work.push(*body);
                        }
                    }
                }
                work.push(pipe.head);
            }
            hir::ExprKind::Map(map) => {
                for entry in map.entries.iter().rev() {
                    work.push(entry.value);
                    work.push(entry.key);
                }
            }
            hir::ExprKind::Apply { callee, arguments } => {
                for argument in arguments.iter().rev() {
                    work.push(*argument);
                }
                work.push(*callee);
            }
            hir::ExprKind::Unary { expr, .. } => work.push(*expr),
            hir::ExprKind::Binary { left, right, .. } => {
                work.push(*right);
                work.push(*left);
            }
            hir::ExprKind::Tuple(elements) => {
                for element in elements.iter().rev() {
                    work.push(*element);
                }
            }
            hir::ExprKind::List(elements) | hir::ExprKind::Set(elements) => {
                for element in elements.iter().rev() {
                    work.push(*element);
                }
            }
            hir::ExprKind::Record(record) => {
                for field in record.fields.iter().rev() {
                    work.push(field.value);
                }
            }
            hir::ExprKind::Projection {
                base: hir::ProjectionBase::Expr(base),
                ..
            } => work.push(*base),
            hir::ExprKind::PatchApply { target, patch } => {
                work.push(*target);
                for entry in patch.entries.iter().rev() {
                    match &entry.instruction.kind {
                        hir::PatchInstructionKind::Replace(expr)
                        | hir::PatchInstructionKind::Store(expr) => work.push(*expr),
                        hir::PatchInstructionKind::Remove => {}
                    }
                }
            }
            hir::ExprKind::PatchLiteral(patch) => {
                for entry in patch.entries.iter().rev() {
                    match &entry.instruction.kind {
                        hir::PatchInstructionKind::Replace(expr)
                        | hir::PatchInstructionKind::Store(expr) => work.push(*expr),
                        hir::PatchInstructionKind::Remove => {}
                    }
                }
            }
            _ => {}
        }
    }
    total
}

#[derive(Clone, Copy)]
enum ReactiveFragmentRole {
    Guard,
    Body,
}

impl ReactiveFragmentRole {
    const fn label(self) -> &'static str {
        match self {
            Self::Guard => "guard",
            Self::Body => "body",
        }
    }

    fn blocked_error(
        self,
        owner: hir::ItemId,
        clause_span: SourceSpan,
        blockers: Box<[hir::GeneralExprBlocker]>,
    ) -> HirRuntimeAdapterError {
        match self {
            Self::Guard => HirRuntimeAdapterError::BlockedReactiveUpdateGuard {
                owner,
                clause_span,
                blockers,
            },
            Self::Body => HirRuntimeAdapterError::BlockedReactiveUpdateBody {
                owner,
                clause_span,
                blockers,
            },
        }
    }
}

fn compile_runtime_expr_fragment(
    module: &hir::Module,
    owner: hir::ItemId,
    clause_span: SourceSpan,
    expr: hir::ExprId,
    expected: &hir::GateType,
    name: Box<str>,
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    role: ReactiveFragmentRole,
) -> Result<HirCompiledRuntimeExpr, HirRuntimeAdapterError> {
    let body = hir::elaborate_runtime_expr_with_env(module, expr, &[], Some(expected)).map_err(
        |blocked| role.blocked_error(owner, clause_span, blocked.blockers.into_boxed_slice()),
    )?;
    let fragment = RuntimeFragmentSpec {
        name: name.clone(),
        owner,
        body_expr: expr,
        parameters: Vec::new(),
        body,
    };
    let lowered = lower_runtime_fragment(module, &fragment).map_err(|error| {
        HirRuntimeAdapterError::ReactiveUpdateFragmentLowering {
            owner,
            clause_span,
            role: role.label(),
            stage: "typed core",
            message: error.to_string().into_boxed_str(),
        }
    })?;
    let lambda = lower_lambda_module(&lowered.module).map_err(|error| {
        HirRuntimeAdapterError::ReactiveUpdateFragmentLowering {
            owner,
            clause_span,
            role: role.label(),
            stage: "typed lambda",
            message: error.to_string().into_boxed_str(),
        }
    })?;
    validate_lambda_module(&lambda).map_err(|error| {
        HirRuntimeAdapterError::ReactiveUpdateFragmentLowering {
            owner,
            clause_span,
            role: role.label(),
            stage: "typed lambda validation",
            message: error.to_string().into_boxed_str(),
        }
    })?;
    let backend = lower_backend_module(&lambda, module).map_err(|error| {
        HirRuntimeAdapterError::ReactiveUpdateFragmentLowering {
            owner,
            clause_span,
            role: role.label(),
            stage: "backend IR",
            message: error.to_string().into_boxed_str(),
        }
    })?;
    validate_program(&backend).map_err(|error| {
        HirRuntimeAdapterError::ReactiveUpdateFragmentLowering {
            owner,
            clause_span,
            role: role.label(),
            stage: "backend validation",
            message: error.to_string().into_boxed_str(),
        }
    })?;
    let entry_item = backend
        .items()
        .iter()
        .find_map(|(item_id, item)| (item.name.as_ref() == name.as_ref()).then_some(item_id))
        .ok_or_else(
            || HirRuntimeAdapterError::ReactiveUpdateFragmentMissingEntry {
                owner,
                clause_span,
                role: role.label(),
                entry_name: name.clone(),
            },
        )?;
    let required_signals = collect_required_fragment_signal_bindings(
        owner,
        clause_span,
        role,
        &lowered.module,
        &backend,
        entry_item,
        public_signals,
    )?;
    Ok(HirCompiledRuntimeExpr {
        backend: Arc::new(backend),
        native_kernels: Arc::new(aivi_backend::NativeKernelArtifactSet::default()),
        entry_item,
        required_signals,
    })
}

fn collect_required_fragment_signal_bindings(
    owner: hir::ItemId,
    clause_span: SourceSpan,
    role: ReactiveFragmentRole,
    core: &aivi_core::Module,
    backend: &BackendProgram,
    entry_item: BackendItemId,
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
) -> Result<Box<[HirCompiledRuntimeExprSignal]>, HirRuntimeAdapterError> {
    let Some(entry) = backend.items().get(entry_item) else {
        return Err(HirRuntimeAdapterError::ReactiveUpdateFragmentMissingEntry {
            owner,
            clause_span,
            role: role.label(),
            entry_name: "<missing-entry-item>".into(),
        });
    };
    let Some(root_kernel) = entry.body else {
        return Ok(Box::default());
    };
    let mut required = BTreeSet::new();
    let mut kernels = vec![root_kernel];
    let mut visited_items = BTreeSet::new();
    while let Some(kernel_id) = kernels.pop() {
        let kernel = &backend.kernels()[kernel_id];
        for &item_id in &kernel.global_items {
            if !visited_items.insert(item_id) {
                continue;
            }
            let item = &backend.items()[item_id];
            match item.kind {
                aivi_backend::ItemKind::Signal(_) => {
                    let hir_item = core.items()[item.origin].origin;
                    let signal = public_signals.get(&hir_item).copied().ok_or_else(|| {
                        HirRuntimeAdapterError::ReactiveUpdateFragmentUnknownSignal {
                            owner,
                            clause_span,
                            role: role.label(),
                            dependency: hir_item,
                        }
                    })?;
                    required.insert((signal, item_id));
                }
                _ => {
                    if let Some(body) = item.body {
                        kernels.push(body);
                    }
                }
            }
        }
    }
    Ok(required
        .into_iter()
        .map(|(signal, backend_item)| HirCompiledRuntimeExprSignal {
            signal,
            backend_item,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

fn resolve_source_option_binding(
    module: &hir::Module,
    owner: hir::ItemId,
    binding: &hir::SourceOptionSignalBinding,
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    errors: &mut Vec<HirRuntimeAdapterError>,
) -> Option<SignalHandle> {
    let target = match binding.signal {
        Some(item) => item,
        None => match resolve_direct_signal_expr(module, binding.expr) {
            Ok(item) => item,
            Err(DirectSignalExprError::UnsupportedExpr) => {
                errors.push(HirRuntimeAdapterError::UnsupportedSourceOptionSignalExpr {
                    owner,
                    option_name: binding.option_name.text().into(),
                    expr: binding.expr,
                });
                return None;
            }
            Err(DirectSignalExprError::ResolvedNonSignal { item, kind }) => {
                errors.push(HirRuntimeAdapterError::SourceOptionBindingNotSignal {
                    owner,
                    option_name: binding.option_name.text().into(),
                    item,
                    kind,
                });
                return None;
            }
        },
    };

    match public_signals.get(&target).copied() {
        Some(handle) => Some(handle),
        None => {
            errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                owner,
                dependency: target,
            });
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectSignalExprError {
    UnsupportedExpr,
    ResolvedNonSignal {
        item: hir::ItemId,
        kind: hir::ItemKind,
    },
}

fn resolve_direct_signal_expr(
    module: &hir::Module,
    expr: hir::ExprId,
) -> Result<hir::ItemId, DirectSignalExprError> {
    let Some(expr) = module.exprs().get(expr) else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    let hir::ExprKind::Name(reference) = &expr.kind else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    let hir::ResolutionState::Resolved(hir::TermResolution::Item(item)) = reference.resolution
    else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    match module.items().get(item) {
        Some(hir::Item::Signal(_)) => Ok(item),
        Some(item_ref) => Err(DirectSignalExprError::ResolvedNonSignal {
            item,
            kind: item_ref.kind(),
        }),
        None => Err(DirectSignalExprError::UnsupportedExpr),
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_hir::{DecodeProgramStep, Item, lower_module};
    use aivi_syntax::parse_module;
    use aivi_typing::{BuiltinSourceProvider, RecurrenceWakeupKind};

    use super::*;

    fn lower_text(path: &str, text: &str) -> hir::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Type(item) if item.name.text() == name => Some(item_id),
                Item::Value(item) if item.name.text() == name => Some(item_id),
                Item::Function(item) if item.name.text() == name => Some(item_id),
                Item::Signal(item) if item.name.text() == name => Some(item_id),
                Item::Class(item) if item.name.text() == name => Some(item_id),
                Item::Domain(item) if item.name.text() == name => Some(item_id),
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    fn runtime_positive_fixture() -> (hir::LoweringResult, HirRuntimeAssembly) {
        let lowered = lower_text(
            "runtime-hir-adapter-positive.aivi",
            r#"
domain Duration over Int
    suffix sec : Int = value => Duration value

fun keep = value=>    value

signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval : Signal Duration = 5sec

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int

signal gatedUsers : Signal Int =
    users
     ?|> True

@recur.timer 5sec
signal retried : Signal Int =
    0
     @|> keep
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "adapter-positive fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("positive runtime fixture should assemble");
        (lowered, assembly)
    }

    #[test]
    fn assembles_signal_graph_and_source_specs_from_hir_reports() {
        let (lowered, assembly) = runtime_positive_fixture();

        let api_host = assembly
            .signal(item_id(lowered.module(), "apiHost"))
            .expect("apiHost signal binding should exist");
        let refresh = assembly
            .signal(item_id(lowered.module(), "refresh"))
            .expect("refresh signal binding should exist");
        let enabled = assembly
            .signal(item_id(lowered.module(), "enabled"))
            .expect("enabled signal binding should exist");
        let poll_interval = assembly
            .signal(item_id(lowered.module(), "pollInterval"))
            .expect("pollInterval signal binding should exist");
        let users_id = item_id(lowered.module(), "users");
        let users = assembly
            .signal(users_id)
            .expect("users signal binding should exist");
        let gated_users_id = item_id(lowered.module(), "gatedUsers");
        let gated_users = assembly
            .signal(gated_users_id)
            .expect("gatedUsers signal binding should exist");
        let retried_id = item_id(lowered.module(), "retried");
        let retried = assembly
            .signal(retried_id)
            .expect("retried signal binding should exist");

        assert!(matches!(users.kind, HirSignalBindingKind::Input { .. }));
        assert!(matches!(
            gated_users.kind,
            HirSignalBindingKind::Derived { .. }
        ));
        assert!(matches!(retried.kind, HirSignalBindingKind::Derived { .. }));
        assert_eq!(gated_users.dependencies(), &[users.signal()]);

        let source = assembly
            .source_by_owner(users_id)
            .expect("users source binding should exist");
        assert_eq!(source.signal, users.signal());
        assert_eq!(
            source.input,
            users.input().expect("users should be input-backed")
        );
        assert_eq!(
            source.spec.provider,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::HttpGet)
        );
        assert_eq!(
            source.spec.reconfiguration_dependencies.as_ref(),
            &[api_host.signal(), poll_interval.signal()]
        );
        assert_eq!(source.spec.explicit_triggers.as_ref(), &[refresh.signal()]);
        assert_eq!(source.spec.active_when, Some(enabled.signal()));
        assert!(matches!(
            source
                .spec
                .decode
                .as_ref()
                .expect("source decode program should be attached")
                .root_step(),
            DecodeProgramStep::Scalar { .. }
        ));

        let gate = assembly
            .gates()
            .iter()
            .find(|gate| gate.site.owner == gated_users_id)
            .expect("gatedUsers gate handoff should exist");
        assert_eq!(gate.owner_signal, gated_users.derived());
        assert!(matches!(gate.plan, HirRuntimeGatePlan::SignalFilter(_)));

        let recurrence = assembly
            .recurrences()
            .iter()
            .find(|node| node.site.owner == retried_id)
            .expect("retried recurrence handoff should exist");
        assert_eq!(recurrence.owner_signal, retried.derived());
        assert_eq!(
            recurrence.wakeup_signal, None,
            "timer recurrence handoff should not invent an upstream wakeup signal"
        );

        let runtime: TaskSourceRuntime<i32, hir::SourceDecodeProgram> = assembly
            .instantiate_runtime()
            .expect("assembled sources should register into a runtime");
        assert!(runtime.source_spec(source.spec.instance).is_some());
        assert_eq!(
            runtime.graph().signal_count(),
            assembly.graph().signal_count()
        );
        let program = assembly.reactive_program();
        let users_node = program
            .signal(users.signal())
            .expect("users signal should be present in the reactive program");
        assert!(matches!(
            users_node.kind(),
            crate::ReactiveSignalNodeKind::Input(input)
                if input.source_instance() == Some(source.spec.instance)
        ));
        let gated_users_node = program
            .signal(gated_users.signal())
            .expect("gatedUsers signal should be present in the reactive program");
        assert!(matches!(
            gated_users_node.kind(),
            crate::ReactiveSignalNodeKind::Derived(info)
                if info.source_input().is_none() && !info.has_recurrence()
        ));
        assert_eq!(gated_users_node.dependencies(), &[users.signal()]);
        assert_eq!(gated_users_node.root_signals(), &[users.signal()]);
        let retried_node = program
            .signal(retried.signal())
            .expect("retried signal should be present in the reactive program");
        assert!(matches!(
            retried_node.kind(),
            crate::ReactiveSignalNodeKind::Derived(info) if info.has_recurrence()
        ));
        assert_eq!(program.signal_count(), assembly.graph().signal_count());
        assert_eq!(program.topo_order().len(), assembly.graph().signal_count());
        assert!(
            program.partitions().len() >= assembly.graph().batches().len(),
            "reactive program partitions should cover at least the scheduled topology batches"
        );
    }

    #[test]
    fn runtime_assembly_roundtrips_through_parts() {
        let (_, assembly) = runtime_positive_fixture();
        let rebuilt = HirRuntimeAssembly::from_parts(assembly.clone().into_parts());
        assert_eq!(rebuilt, assembly);
    }

    #[test]
    fn capability_handle_anchors_do_not_assemble_as_runtime_signals() {
        let lowered = lower_text(
            "runtime-hir-capability-handles.aivi",
            r#"
type FsSource = Unit
type FsError = Text

signal projectRoot : Signal Text = "/tmp/demo"

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
value cleanup : Task Text Unit = files.delete "cache.txt"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "capability handle runtime fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("capability handle runtime fixture should assemble");

        let files_id = item_id(lowered.module(), "files");
        let config_id = item_id(lowered.module(), "config");
        let cleanup_id = item_id(lowered.module(), "cleanup");

        assert!(
            assembly.owner(files_id).is_none(),
            "compile-time capability handles should not allocate runtime owners"
        );
        assert!(
            assembly.signal(files_id).is_none(),
            "compile-time capability handles should not assemble as runtime signals"
        );
        assert!(
            assembly.source_by_owner(config_id).is_some(),
            "capability source operations should still assemble as ordinary runtime sources"
        );
        assert!(
            assembly.task_by_owner(cleanup_id).is_some(),
            "capability value commands should still assemble as ordinary runtime tasks"
        );
    }

    #[test]
    fn custom_capability_operations_assemble_as_member_qualified_custom_sources() {
        let lowered = lower_text(
            "runtime-hir-custom-capability-operations.aivi",
            r#"
type FeedSource = Unit

signal root = "/tmp/demo"

provider custom.feed
    argument path: Text
    operation read : Text -> Signal Int

@source custom.feed root
signal feed : FeedSource

signal config : Signal Int = feed.read "config"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "custom capability operation fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("custom capability operation fixture should assemble");
        let config_id = item_id(lowered.module(), "config");
        let binding = assembly
            .source_by_owner(config_id)
            .expect("custom capability operation should assemble as a source binding");
        assert_eq!(
            binding.spec.provider,
            RuntimeSourceProvider::custom("custom.feed.read")
        );
    }

    #[test]
    fn assembles_db_live_refresh_from_changed_projection() {
        let lowered = lower_text(
            "runtime-hir-adapter-db-live-changed.aivi",
            r#"
type TableRef A = {
    changed: Signal Unit
}

signal usersChanged : Signal Unit

value users : TableRef Int = {
    changed: usersChanged
}

@source db.live with {
    refreshOn: users.changed
}
signal rows : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "db.live changed projection fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("db.live changed projection fixture should assemble");
        let changed = assembly
            .signal(item_id(lowered.module(), "usersChanged"))
            .expect("usersChanged signal binding should exist");
        let rows_id = item_id(lowered.module(), "rows");
        let rows = assembly
            .source_by_owner(rows_id)
            .expect("rows source binding should exist");

        assert_eq!(
            rows.spec.provider,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbLive)
        );
        assert_eq!(rows.spec.explicit_triggers.as_ref(), &[changed.signal()]);
    }

    #[test]
    fn assembles_task_specs_from_task_values() {
        let lowered = lower_text(
            "runtime-hir-adapter-task.aivi",
            r#"
domain Retry over Int
    suffix times : Int = value => Retry value

fun keep = value=>    value

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> keep
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "task fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly =
            assemble_hir_runtime(lowered.module()).expect("task fixture should assemble");
        let retried_id = item_id(lowered.module(), "retried");
        let task = assembly
            .task_by_owner(retried_id)
            .expect("retried task binding should exist");

        assert_eq!(task.owner, retried_id);
        assert_eq!(
            task.spec
                .wakeup
                .expect("task wakeup should be preserved")
                .kind(),
            RecurrenceWakeupKind::Backoff
        );

        let runtime: TaskSourceRuntime<i32, hir::SourceDecodeProgram> = assembly
            .instantiate_runtime()
            .expect("assembled tasks should register into a runtime");
        assert!(runtime.task_spec(task.spec.instance).is_some());
        assert_eq!(
            runtime.graph().signal_count(),
            assembly.graph().signal_count()
        );
    }

    #[test]
    fn bodyless_sources_and_accumulate_signals_keep_runtime_roles_separate() {
        let lowered = lower_text(
            "runtime-hir-adapter-bodyless-source-accumulate.aivi",
            r#"
fun step:Int = n:Int current:Int=>    n

signal enabled = True

@source http.get "/users" with {
    activeWhen: enabled
}
signal userEvents : Signal Int

signal gated : Signal Int =
    userEvents
     +|> 0 step
"#,
        );
        assert!(
            !lowered.has_errors(),
            "bodyless source accumulate fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("bodyless source accumulate fixture should assemble");
        let user_events_id = item_id(lowered.module(), "userEvents");
        let gated_id = item_id(lowered.module(), "gated");
        let user_events = assembly
            .signal(user_events_id)
            .expect("userEvents signal binding should exist");
        let gated = assembly
            .signal(gated_id)
            .expect("gated signal binding should exist");
        let source = assembly
            .source_by_owner(user_events_id)
            .expect("userEvents source binding should exist");

        let derived = gated
            .derived()
            .expect("accumulate-derived signal should expose a public derived handle");
        assert_eq!(source.signal, user_events.signal());
        assert_eq!(
            source.input,
            user_events
                .input()
                .expect("raw source signal should stay input-backed")
        );
        assert!(
            gated.source_input.is_none(),
            "derived accumulate signals should not allocate their own source input handle"
        );
        assert_eq!(
            gated.dependencies(),
            &[user_events.signal()],
            "accumulate-derived signals should depend on the raw source signal"
        );
        assert_eq!(
            source.spec.active_when,
            Some(
                assembly
                    .signal(item_id(lowered.module(), "enabled"))
                    .expect("enabled signal binding should exist")
                    .signal()
            )
        );
        let recurrence = assembly
            .recurrences()
            .iter()
            .find(|node| node.site.owner == gated_id)
            .expect("gated recurrence handoff should exist");
        assert_eq!(recurrence.owner_signal, Some(derived));
        assert_eq!(
            recurrence.wakeup_signal,
            Some(user_events_id),
            "accumulate recurrence handoff should preserve its upstream wakeup signal"
        );
    }

    #[test]
    fn assembles_reactive_signal_clauses_into_runtime_bindings() {
        let lowered = lower_text(
            "runtime-hir-adapter-reactive-updates.aivi",
            r#"
signal left = 20
signal right = 22
signal ready = True
signal enabled = True

signal readyAndEnabled = ready and enabled

signal total : Signal Int = ready | readyAndEnabled
  ||> readyAndEnabled True => left + right + 1
  ||> ready True => left + right
  ||> _ => 0

signal doubled = total + total
"#,
        );
        assert!(
            !lowered.has_errors(),
            "reactive-update fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("reactive-update fixture should assemble into runtime bindings");
        let total = assembly
            .signal(item_id(lowered.module(), "total"))
            .expect("total signal binding should exist");
        let doubled = assembly
            .signal(item_id(lowered.module(), "doubled"))
            .expect("doubled signal binding should exist");

        assert!(matches!(total.kind, HirSignalBindingKind::Reactive { .. }));
        assert!(
            !total.reactive_updates().is_empty(),
            "total should have reactive updates from merge arms"
        );
        assert!(
            total.reactive_seed_dependencies().is_empty(),
            "constant seed should not introduce signal dependencies"
        );
        for clause in total.reactive_updates() {
            assert_eq!(
                assembly
                    .graph()
                    .reactive_clause(clause.clause)
                    .expect("reactive clause handle should resolve")
                    .target(),
                total.signal()
            );
        }
        // doubled depends on total
        assert!(doubled.dependencies().contains(&total.signal()));
    }

    #[test]
    fn assembles_pattern_armed_reactive_updates_into_runtime_bindings() {
        let lowered = lower_text(
            "runtime-hir-adapter-pattern-reactive-updates.aivi",
            r#"
type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down

signal heading : Signal Direction = event
  ||> Turn dir => dir
  ||> _ => Up

signal tickSeen : Signal Bool = event
  ||> Tick => True
  ||> _ => False
"#,
        );
        assert!(
            !lowered.has_errors(),
            "pattern-armed reactive update fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("pattern-armed reactive update fixture should assemble");
        let event = assembly
            .signal(item_id(lowered.module(), "event"))
            .expect("event signal binding should exist");
        let heading = assembly
            .signal(item_id(lowered.module(), "heading"))
            .expect("heading signal binding should exist");
        let tick_seen = assembly
            .signal(item_id(lowered.module(), "tickSeen"))
            .expect("tickSeen signal binding should exist");

        assert!(matches!(
            heading.kind,
            HirSignalBindingKind::Reactive { .. }
        ));
        assert!(matches!(
            tick_seen.kind,
            HirSignalBindingKind::Reactive { .. }
        ));
        assert!(heading.dependencies().contains(&event.signal()));
        assert!(tick_seen.dependencies().contains(&event.signal()));
        assert!(
            !heading.reactive_updates().is_empty(),
            "heading should have reactive updates from merge arms"
        );
        assert!(
            !tick_seen.reactive_updates().is_empty(),
            "tickSeen should have reactive updates from merge arms"
        );
        assert_eq!(
            heading.reactive_updates()[0].body_mode,
            hir::ReactiveUpdateBodyMode::OptionalPayload
        );
        assert_eq!(
            tick_seen.reactive_updates()[0].body_mode,
            hir::ReactiveUpdateBodyMode::OptionalPayload
        );
    }

    #[test]
    fn assembles_source_pattern_reactive_updates_into_runtime_bindings() {
        let lowered = lower_text(
            "runtime-hir-adapter-source-pattern-reactive-updates.aivi",
            r#"
provider custom.ready
    wakeup: providerTrigger

@source custom.ready
signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
        );
        assert!(
            !lowered.has_errors(),
            "source-pattern reactive update fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("source-pattern reactive update fixture should assemble");
        let ready = assembly
            .signal(item_id(lowered.module(), "ready"))
            .expect("ready signal binding should exist");
        let total = assembly
            .signal(item_id(lowered.module(), "total"))
            .expect("total signal binding should exist");

        assert!(matches!(total.kind, HirSignalBindingKind::Reactive { .. }));
        assert!(total.dependencies().contains(&ready.signal()));
        assert!(
            !total.reactive_updates().is_empty(),
            "total should have reactive updates from merge arms"
        );
        assert_eq!(
            total.reactive_updates()[0].body_mode,
            hir::ReactiveUpdateBodyMode::OptionalPayload
        );
    }

    #[test]
    fn lowers_builtin_dbus_source_providers_into_runtime_specs() {
        let lowered = lower_text(
            "runtime-hir-adapter-dbus.aivi",
            r#"
@source dbus.signal "/org/aivi/Test" with {
    interface: "org.aivi.Test"
    member: "Ping"
}
signal inbound : Signal Text
"#,
        );
        assert!(
            !lowered.has_errors(),
            "dbus fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("dbus fixture should assemble into a runtime");
        let inbound_id = item_id(lowered.module(), "inbound");
        let source = assembly
            .source_by_owner(inbound_id)
            .expect("dbus source binding should exist");

        assert_eq!(
            source.spec.provider,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbusSignal)
        );
    }

    #[test]
    fn reactive_program_keeps_same_block_parameterized_from_signal_dependencies() {
        let lowered = lower_text(
            "runtime-hir-adapter-parameterized-from-same-block-signals.aivi",
            r#"
type State = { score: Int, ready: Bool }

signal state : Signal State = { score: 1, ready: True }

from state = {
    score: .score
    ready: .ready

    type Int -> Bool
    atLeast threshold: ready and score >= threshold
}

signal thresholdMet : Signal Bool = atLeast 0
"#,
        );
        assert!(
            !lowered.has_errors(),
            "parameterized same-block selector fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("parameterized same-block selector fixture should assemble");
        let state = assembly
            .signal(item_id(lowered.module(), "state"))
            .expect("state signal binding should exist");
        let threshold_met = assembly
            .signal(item_id(lowered.module(), "thresholdMet"))
            .expect("thresholdMet signal binding should exist");
        let node = assembly
            .reactive_program()
            .signal(threshold_met.signal())
            .expect("thresholdMet should appear in the reactive program");

        assert!(matches!(
            node.kind(),
            crate::ReactiveSignalNodeKind::Derived(_)
        ));
        assert!(
            node.dependencies().contains(&state.signal()),
            "thresholdMet should retain its upstream state signal dependency"
        );
        assert!(
            node.root_signals().contains(&state.signal()),
            "thresholdMet should trace back to the state signal as a root"
        );
        assert!(
            node.topo_index()
                > assembly
                    .reactive_program()
                    .signal(state.signal())
                    .expect("state should appear in the reactive program")
                    .topo_index(),
            "thresholdMet should evaluate after its upstream state signal"
        );
    }

    #[test]
    fn reactive_program_splits_disjoint_same_batch_signals_into_root_partitions() {
        let lowered = lower_text(
            "runtime-hir-adapter-root-partitions.aivi",
            r#"
signal leftInput = 1
signal rightInput = 10

signal left = leftInput + 1
signal right = rightInput + 1
signal total = left + right
"#,
        );
        assert!(
            !lowered.has_errors(),
            "root-partition fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly =
            assemble_hir_runtime(lowered.module()).expect("root-partition fixture should assemble");
        let program = assembly.reactive_program();
        let left_input = assembly
            .signal(item_id(lowered.module(), "leftInput"))
            .expect("leftInput signal binding should exist")
            .signal();
        let right_input = assembly
            .signal(item_id(lowered.module(), "rightInput"))
            .expect("rightInput signal binding should exist")
            .signal();
        let left = assembly
            .signal(item_id(lowered.module(), "left"))
            .expect("left signal binding should exist")
            .signal();
        let right = assembly
            .signal(item_id(lowered.module(), "right"))
            .expect("right signal binding should exist")
            .signal();
        let total = assembly
            .signal(item_id(lowered.module(), "total"))
            .expect("total signal binding should exist")
            .signal();

        let left_node = program
            .signal(left)
            .expect("left should appear in the reactive program");
        let right_node = program
            .signal(right)
            .expect("right should appear in the reactive program");
        let total_node = program
            .signal(total)
            .expect("total should appear in the reactive program");

        assert_ne!(
            left_node.partition(),
            right_node.partition(),
            "disjoint same-batch derived signals should land in separate partitions",
        );
        let left_partition = program
            .partition(left_node.partition())
            .expect("left partition should exist");
        let right_partition = program
            .partition(right_node.partition())
            .expect("right partition should exist");
        let total_partition = program
            .partition(total_node.partition())
            .expect("total partition should exist");

        assert_eq!(left_partition.root_signals(), &[left_input]);
        assert_eq!(right_partition.root_signals(), &[right_input]);
        assert_eq!(left_partition.signals(), &[left]);
        assert_eq!(right_partition.signals(), &[right]);
        assert_eq!(total_partition.root_signals(), &[left_input, right_input]);
        assert_eq!(total_partition.signals(), &[total]);
        assert_eq!(
            &program.topo_order()[left_partition.topo_range().clone()],
            left_partition.signals(),
            "partition topo slices should remain contiguous in topo order",
        );
        assert_eq!(
            &program.topo_order()[right_partition.topo_range().clone()],
            right_partition.signals(),
            "partition topo slices should remain contiguous in topo order",
        );
    }

    #[test]
    fn reports_unsupported_source_option_expressions_explicitly() {
        let lowered = lower_text(
            "runtime-hir-adapter-unsupported-source-option.aivi",
            r#"
signal enabled = True
signal other = False

@source http.get "/users" with {
    activeWhen: enabled and other
}
signal users : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "unsupported-option fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let users_id = item_id(lowered.module(), "users");
        let errors = assemble_hir_runtime(lowered.module())
            .expect_err("composite activeWhen expressions should stay explicit adapter errors");
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            HirRuntimeAdapterError::UnsupportedSourceOptionSignalExpr {
                owner,
                option_name,
                ..
            } if *owner == users_id && option_name.as_ref() == "activeWhen"
        )));
    }

    #[test]
    fn forwards_blocked_decode_programs_into_adapter_failures() {
        let lowered = lower_text(
            "runtime-hir-adapter-blocked-decode.aivi",
            r#"
@source custom.feed
signal bad : Signal (Signal Int)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "blocked-decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let bad_id = item_id(lowered.module(), "bad");
        let errors = assemble_hir_runtime(lowered.module())
            .expect_err("blocked source decode programs should stop assembly");
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            HirRuntimeAdapterError::BlockedSourceDecodeProgram { owner, .. }
            if *owner == bad_id
        )));
    }

    #[test]
    fn preserves_gate_outcomes_without_rederiving_them() {
        let lowered = lower_text(
            "runtime-hir-adapter-gate-preservation.aivi",
            r#"
value maybeOne:Option Int =
    1
     ?|> True
"#,
        );
        assert!(
            !lowered.has_errors(),
            "gate-preservation fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = HirRuntimeAssemblyBuilder::new(
            lowered.module(),
            &hir::elaborate_source_lifecycles(lowered.module()),
            &hir::generate_source_decode_programs(lowered.module()),
            &hir::elaborate_recurrences(lowered.module()),
            &hir::elaborate_gates(lowered.module()),
        )
        .build()
        .expect("gate-preservation fixture should assemble");
        let gate = assembly
            .gates()
            .iter()
            .find(|gate| gate.site.owner == item_id(lowered.module(), "maybeOne"))
            .expect("ordinary gate handoff should be preserved");
        assert!(matches!(gate.plan, HirRuntimeGatePlan::Ordinary(_)));
    }
}
