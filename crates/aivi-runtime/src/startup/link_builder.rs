struct LinkArtifacts {
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    db_changed_routes: Box<[LinkedDbChangedRoute]>,
}

struct LinkBuilder<'a> {
    assembly: &'a HirRuntimeAssembly,
    core: &'a core::Module,
    backend: &'a BackendProgram,
    errors: Vec<BackendRuntimeLinkError>,
    core_to_hir: BTreeMap<core::ItemId, hir::ItemId>,
    hir_to_backend: BTreeMap<hir::ItemId, BackendItemId>,
    backend_to_hir: BTreeMap<BackendItemId, hir::ItemId>,
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    db_changed_routes: Vec<LinkedDbChangedRoute>,
}

impl<'a> LinkBuilder<'a> {
    fn new(
        assembly: &'a HirRuntimeAssembly,
        core: &'a core::Module,
        backend: &'a BackendProgram,
    ) -> Self {
        Self {
            assembly,
            core,
            backend,
            errors: Vec::new(),
            core_to_hir: BTreeMap::new(),
            hir_to_backend: BTreeMap::new(),
            backend_to_hir: BTreeMap::new(),
            signal_items_by_handle: BTreeMap::new(),
            runtime_signal_by_item: BTreeMap::new(),
            derived_signals: BTreeMap::new(),
            reactive_signals: BTreeMap::new(),
            reactive_clauses: BTreeMap::new(),
            linked_recurrence_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            task_bindings: BTreeMap::new(),
            db_changed_routes: Vec::new(),
        }
    }

    fn build(&mut self) -> Result<LinkArtifacts, BackendRuntimeLinkErrors> {
        self.index_origins();
        self.index_signal_items();
        self.link_sources();
        self.link_tasks();
        self.link_reactive_signals();
        self.link_db_changed_routes();
        self.link_derived_signals();
        if self.errors.is_empty() {
            Ok(LinkArtifacts {
                signal_items_by_handle: std::mem::take(&mut self.signal_items_by_handle),
                runtime_signal_by_item: std::mem::take(&mut self.runtime_signal_by_item),
                derived_signals: std::mem::take(&mut self.derived_signals),
                reactive_signals: std::mem::take(&mut self.reactive_signals),
                reactive_clauses: std::mem::take(&mut self.reactive_clauses),
                linked_recurrence_signals: std::mem::take(&mut self.linked_recurrence_signals),
                source_bindings: std::mem::take(&mut self.source_bindings),
                task_bindings: std::mem::take(&mut self.task_bindings),
                db_changed_routes: std::mem::take(&mut self.db_changed_routes).into_boxed_slice(),
            })
        } else {
            Err(BackendRuntimeLinkErrors::new(std::mem::take(
                &mut self.errors,
            )))
        }
    }

    fn index_origins(&mut self) {
        for (core_id, item) in self.core.items().iter() {
            self.core_to_hir.insert(core_id, item.origin);
        }
        for (backend_item, item) in self.backend.items().iter() {
            let Some(&hir_item) = self.core_to_hir.get(&item.origin) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingCoreItemOrigin {
                        backend_item,
                        core_item: item.origin,
                    });
                continue;
            };
            if let Some(previous) = self.hir_to_backend.insert(hir_item, backend_item) {
                self.errors
                    .push(BackendRuntimeLinkError::DuplicateBackendOrigin {
                        item: hir_item,
                        first: previous,
                        second: backend_item,
                    });
            }
            self.backend_to_hir.insert(backend_item, hir_item);
        }
    }

    fn index_signal_items(&mut self) {
        for binding in self.assembly.signals() {
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            self.signal_items_by_handle
                .insert(binding.signal(), backend_item);
            self.runtime_signal_by_item
                .insert(backend_item, binding.signal());
        }
    }

    fn link_sources(&mut self) {
        for source in self.assembly.sources() {
            let Some(&backend_owner) = self.hir_to_backend.get(&source.owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let Some(owner_handle) = self
                .assembly
                .owner(source.owner)
                .map(|binding| binding.handle)
            else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingRuntimeOwner {
                        owner: source.owner,
                    });
                continue;
            };
            let Some(backend_item) = self.backend.items().get(backend_owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let BackendItemKind::Signal(info) = &backend_item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let Some(backend_source_id) = info.source else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendSource {
                        owner: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let backend_source = &self.backend.sources()[backend_source_id];
            if backend_source.instance.as_raw() != source.spec.instance.as_raw() {
                self.errors
                    .push(BackendRuntimeLinkError::SourceInstanceMismatch {
                        owner: source.owner,
                        runtime: source.spec.instance,
                        backend: backend_source.instance,
                    });
                continue;
            }

            let arguments = backend_source
                .arguments
                .iter()
                .map(|argument| LinkedSourceArgument {
                    kernel: argument.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, argument.kernel),
                })
                .collect::<Vec<_>>();
            let options = backend_source
                .options
                .iter()
                .filter(|option| {
                    !matches!(
                        option.option_name.as_ref(),
                        "decode" | "refreshOn" | "reloadOn" | "restartOn" | "activeWhen"
                    )
                })
                .map(|option| LinkedSourceOption {
                    option_name: option.option_name.clone(),
                    kernel: option.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, option.kernel),
                })
                .collect::<Vec<_>>();

            self.source_bindings.insert(
                source.spec.instance,
                LinkedSourceBinding {
                    owner: source.owner,
                    owner_handle,
                    signal: source.signal,
                    input: source.input,
                    instance: source.spec.instance,
                    backend_owner,
                    backend_source: backend_source_id,
                    arguments: arguments.into_boxed_slice(),
                    options: options.into_boxed_slice(),
                },
            );
        }
    }

    fn link_tasks(&mut self) {
        for task in self.assembly.tasks() {
            let Some(&backend_item) = self.hir_to_backend.get(&task.owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: task.owner });
                continue;
            };
            let Some(owner_handle) = self
                .assembly
                .owner(task.owner)
                .map(|binding| binding.handle)
            else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingRuntimeOwner { owner: task.owner });
                continue;
            };
            let Some(item) = self.backend.items().get(backend_item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: task.owner });
                continue;
            };
            debug_assert!(
                item.parameters.is_empty(),
                "runtime task bindings currently originate from parameterless top-level values"
            );
            let execution = if !item.parameters.is_empty() {
                LinkedTaskExecutionBinding::Blocked(
                    LinkedTaskExecutionBlocker::UnsupportedParameters {
                        parameter_count: item.parameters.len(),
                    },
                )
            } else if let Some(kernel) = item.body {
                LinkedTaskExecutionBinding::Ready {
                    kernel,
                    required_signals: self.collect_required_signal_items(task.owner, kernel),
                }
            } else {
                LinkedTaskExecutionBinding::Blocked(LinkedTaskExecutionBlocker::MissingLoweredBody)
            };
            self.task_bindings.insert(
                task.spec.instance,
                LinkedTaskBinding {
                    owner: task.owner,
                    owner_handle,
                    input: task.input,
                    instance: task.spec.instance,
                    backend_item,
                    execution,
                },
            );
        }
    }

    fn link_reactive_signals(&mut self) {
        for binding in self.assembly.signals() {
            let Some(reactive) = binding.reactive_signal() else {
                continue;
            };
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(info) = &item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };
            if !item.pipelines.is_empty()
                && !self.supported_body_backed_reactive_signal_pipelines(item)
            {
                self.errors
                    .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                        item: binding.item,
                        count: item.pipelines.len(),
                    });
                continue;
            }
            let pipeline_ids = item
                .pipelines
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let has_seed_body = item.body.is_some();
            let pipeline_signals = self
                .collect_pipeline_signal_handles(binding.item, pipeline_ids.as_ref())
                .into_boxed_slice();
            let seed_eval_lane =
                self.kernel_eval_lane(self.backend, info.body_kernel, info.dependency_layouts.as_slice());
            self.reactive_signals.insert(
                reactive,
                LinkedReactiveSignal {
                    item: binding.item,
                    signal: reactive,
                    backend_item,
                    has_seed_body,
                    body_kernel: info.body_kernel,
                    seed_eval_lane,
                    dependency_items: info.dependencies.clone().into_boxed_slice(),
                    dependency_layouts: info.dependency_layouts.clone().into_boxed_slice(),
                    pipeline_signals,
                    pipeline_ids: pipeline_ids.clone(),
                },
            );
            for clause in binding.reactive_updates() {
                self.reactive_clauses.insert(
                    clause.clause,
                    LinkedReactiveClause {
                        owner: binding.item,
                        target: reactive,
                        clause: clause.clause,
                        pipeline_ids: pipeline_ids.clone(),
                        body_mode: clause.body_mode,
                        guard_eval_lane: self.fragment_eval_lane(&clause.compiled_guard),
                        body_eval_lane: self.fragment_eval_lane(&clause.compiled_body),
                        compiled_guard: clause.compiled_guard.clone(),
                        compiled_body: clause.compiled_body.clone(),
                    },
                );
            }
        }
    }

    fn link_db_changed_routes(&mut self) {
        for binding in self.assembly.db_changed_bindings() {
            let Some(changed_input) = self
                .assembly
                .signal(binding.changed_signal)
                .and_then(|signal| signal.input())
            else {
                continue;
            };
            let table = if let Some(signal) = self.assembly.signal(binding.table_item) {
                LinkedDbChangedRouteTable::Signal {
                    signal: signal.signal(),
                }
            } else {
                let Some(&backend_item) = self.hir_to_backend.get(&binding.table_item) else {
                    continue;
                };
                let Some(item) = self.backend.items().get(backend_item) else {
                    continue;
                };
                let Some(body) = item.body else {
                    continue;
                };
                if !item.parameters.is_empty() {
                    continue;
                }
                LinkedDbChangedRouteTable::Value {
                    owner: binding.table_item,
                    backend_item,
                    required_signals: self.collect_required_signal_items(binding.table_item, body),
                    changed_signal_item: self.hir_to_backend.get(&binding.changed_signal).copied(),
                }
            };
            self.db_changed_routes.push(LinkedDbChangedRoute {
                changed_input,
                table,
            });
        }
    }

    fn link_derived_signals(&mut self) {
        for binding in self.assembly.signals() {
            let Some(derived) = binding.derived() else {
                continue;
            };
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(info) = &item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };
            if let Some(pipeline_id) = item
                .pipelines
                .iter()
                .copied()
                .find(|&pid| self.backend.pipelines()[pid].recurrence.is_some())
            {
                let Some(recurrence_binding) = self
                    .assembly
                    .recurrences()
                    .iter()
                    .find(|recurrence| recurrence.site.owner == binding.item)
                else {
                    self.errors
                        .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                            item: binding.item,
                            count: item.pipelines.len(),
                        });
                    continue;
                };
                let Some(wakeup_dependency_index) =
                    self.recurrence_wakeup_dependency_index(binding, &recurrence_binding.plan)
                else {
                    self.errors
                        .push(BackendRuntimeLinkError::MissingRecurrenceWakeupDependency {
                            item: binding.item,
                        });
                    continue;
                };

                let recurrence = self.backend.pipelines()[pipeline_id]
                    .recurrence
                    .as_ref()
                    .expect("selected recurrence pipeline should carry recurrence metadata");
                let seed_kernel = recurrence.seed;
                let mut step_kernels = Vec::with_capacity(1 + recurrence.steps.len());
                step_kernels.push(recurrence.start.kernel);
                step_kernels.extend(recurrence.steps.iter().map(|step| step.kernel));
                let step_kernels = step_kernels.into_boxed_slice();
                let all_kernels: Vec<KernelId> = std::iter::once(seed_kernel)
                    .chain(step_kernels.iter().copied())
                    .collect();
                let required = self.collect_recurrence_signal_items(binding.item, &all_kernels);
                let dependency_items = binding
                    .dependencies()
                    .iter()
                    .filter_map(|signal| self.signal_items_by_handle.get(signal).copied())
                    .collect::<Vec<_>>();
                if !same_items(&required, &dependency_items) {
                    self.errors
                        .push(BackendRuntimeLinkError::SignalRequirementMismatch {
                            item: binding.item,
                            declared: dependency_items.clone().into_boxed_slice(),
                            required,
                        });
                    continue;
                }

                self.linked_recurrence_signals.insert(
                    derived,
                    LinkedRecurrenceSignal {
                        item: binding.item,
                        signal: derived,
                        backend_item,
                        wakeup_dependency_index,
                        seed_kernel,
                        step_kernels,
                        dependency_items: dependency_items.into_boxed_slice(),
                        pipeline_ids: item
                            .pipelines
                            .iter()
                            .copied()
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    },
                );
                continue;
            }
            if !item.pipelines.is_empty()
                && !self.supported_body_backed_derived_signal_pipelines(item)
            {
                self.errors
                    .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                        item: binding.item,
                        count: item.pipelines.len(),
                    });
                continue;
            }
            let body = item.body;
            let body_kernel = info.body_kernel;
            if body.is_none() && body_kernel.is_none() {
                self.errors
                    .push(BackendRuntimeLinkError::MissingSignalBody {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            }

            let required = if let Some(body) = body {
                self.collect_required_signal_items(binding.item, body)
            } else {
                info.dependencies.clone().into_boxed_slice()
            };
            let declared = info.dependencies.clone().into_boxed_slice();
            if !same_items(&required, &declared) {
                self.errors
                    .push(BackendRuntimeLinkError::SignalRequirementMismatch {
                        item: binding.item,
                        declared,
                        required,
                    });
                continue;
            }

            let backend_dependencies = info
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    self.runtime_signal_for_backend_item(binding.item, *dependency)
                })
                .collect::<Vec<_>>();
            let temporal_helpers = self.collect_linked_temporal_helpers(binding, item);
            let mut expected_runtime_dependencies = backend_dependencies.clone();
            if let Some(source_input) = binding.source_input {
                expected_runtime_dependencies.push(source_input.as_signal());
            }
            expected_runtime_dependencies.extend(
                temporal_helpers
                    .iter()
                    .map(|helper| helper.input.as_signal()),
            );
            if expected_runtime_dependencies.as_slice() != binding.dependencies() {
                self.errors
                    .push(BackendRuntimeLinkError::SignalDependencyMismatch {
                        item: binding.item,
                        runtime: binding.dependencies().to_vec().into_boxed_slice(),
                        backend: expected_runtime_dependencies.into_boxed_slice(),
                    });
                continue;
            }

            let dependency_layouts = info.dependency_layouts.clone().into_boxed_slice();
            let eval_lane =
                self.kernel_eval_lane(self.backend, body_kernel, dependency_layouts.as_ref());

            self.derived_signals.insert(
                derived,
                LinkedDerivedSignal {
                    item: binding.item,
                    signal: derived,
                    backend_item,
                    body_kernel,
                    eval_lane,
                    dependency_items: info.dependencies.clone().into_boxed_slice(),
                    dependency_layouts,
                    source_input: binding.source_input,
                    pipeline_ids: item
                        .pipelines
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                    temporal_helpers: temporal_helpers.into_boxed_slice(),
                },
            );
        }
    }

    fn recurrence_wakeup_dependency_index(
        &self,
        binding: &crate::hir_adapter::HirSignalBinding,
        plan: &hir::RecurrenceNodePlan,
    ) -> Option<usize> {
        if let Some(wakeup_signal) = plan.wakeup_signal {
            let wakeup_handle = self.assembly.signal(wakeup_signal)?.signal();
            return binding
                .dependencies()
                .iter()
                .position(|&signal| signal == wakeup_handle);
        }
        let source_input = binding.source_input?;
        binding
            .dependencies()
            .iter()
            .position(|&signal| signal == source_input.as_signal())
    }

    fn runtime_signal_for_backend_item(
        &mut self,
        owner: hir::ItemId,
        dependency: BackendItemId,
    ) -> Option<SignalHandle> {
        if let Some(signal) = self.runtime_signal_by_item.get(&dependency).copied() {
            return Some(signal);
        }
        self.errors
            .push(BackendRuntimeLinkError::MissingRuntimeSignalDependency { owner, dependency });
        None
    }

    fn collect_recurrence_signal_items(
        &mut self,
        owner: hir::ItemId,
        kernels: &[KernelId],
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernel_queue = kernels.to_vec();
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernel_queue.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for &item_id in &kernel.global_items {
                if !visited_items.insert(item_id) {
                    continue;
                }
                let item = &self.backend.items()[item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(item_id);
                    }
                    _ => {
                        if item.name.starts_with("__aivi_") {
                            // Ambient prelude items are runtime-interpreted, not compiled.
                            continue;
                        }
                        if let Some(body) = item.body {
                            kernel_queue.push(body);
                        } else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: item_id,
                                });
                        }
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }

    fn collect_required_signal_items(
        &mut self,
        owner: hir::ItemId,
        root: KernelId,
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernels = vec![root];
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernels.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for item_id in &kernel.global_items {
                if !visited_items.insert(*item_id) {
                    continue;
                }
                let item = &self.backend.items()[*item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(*item_id);
                    }
                    _ => {
                        if item.name.starts_with("__aivi_") {
                            continue;
                        }
                        let Some(body) = item.body else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: *item_id,
                                });
                            continue;
                        };
                        kernels.push(body);
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }

    fn collect_required_signal_items_for_kernels(
        &mut self,
        owner: hir::ItemId,
        kernels: Vec<KernelId>,
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernels = kernels;
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernels.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for item_id in &kernel.global_items {
                if !visited_items.insert(*item_id) {
                    continue;
                }
                let item = &self.backend.items()[*item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(*item_id);
                    }
                    _ => {
                        if item.name.starts_with("__aivi_") {
                            continue;
                        }
                        let Some(body) = item.body else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: *item_id,
                                });
                            continue;
                        };
                        kernels.push(body);
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }

    fn kernel_eval_lane(
        &self,
        backend: &BackendProgram,
        kernel: Option<KernelId>,
        dependency_layouts: &[aivi_backend::LayoutId],
    ) -> LinkedEvalLane {
        let Some(kernel) = kernel else {
            return LinkedEvalLane::Fallback;
        };
        let Some(native_plan) = aivi_backend::NativeKernelPlan::compile(backend, kernel) else {
            return LinkedEvalLane::Fallback;
        };
        LinkedEvalLane::Native(LinkedNativeKernelEval {
            kernel,
            dependency_layouts: dependency_layouts.to_vec().into_boxed_slice(),
            result_layout: native_plan.result_layout(),
        })
    }

    fn fragment_eval_lane(&self, fragment: &HirCompiledRuntimeExpr) -> LinkedEvalLane {
        let Some(kernel) = fragment
            .backend
            .items()
            .get(fragment.entry_item)
            .and_then(|item| item.body)
        else {
            return LinkedEvalLane::Fallback;
        };
        self.kernel_eval_lane(fragment.backend.as_ref(), Some(kernel), &[])
    }

    fn collect_pipeline_signal_handles(
        &mut self,
        owner: hir::ItemId,
        pipeline_ids: &[BackendPipelineId],
    ) -> Vec<SignalHandle> {
        let mut kernels = Vec::new();
        for &pipeline_id in pipeline_ids {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::Ordinary {
                        when_true,
                        when_false,
                    }) => {
                        kernels.push(*when_true);
                        kernels.push(*when_false);
                    }
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate, ..
                    }) => {
                        kernels.push(*predicate);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Previous { seed, .. }) => {
                        kernels.push(*seed);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::DiffFunction {
                        diff, ..
                    }) => {
                        kernels.push(*diff);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::DiffSeed { seed, .. }) => {
                        kernels.push(*seed);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Delay {
                        duration, ..
                    }) => {
                        kernels.push(*duration);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Burst {
                        every,
                        count,
                        ..
                    }) => {
                        kernels.push(*every);
                        kernels.push(*count);
                    }
                    BackendStageKind::Fanout(fanout) => {
                        kernels.push(fanout.map);
                        kernels.extend(fanout.filters.iter().map(|filter| filter.predicate));
                        if let Some(join) = &fanout.join {
                            kernels.push(join.kernel);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {}
                }
            }
        }
        self.collect_required_signal_items_for_kernels(owner, kernels)
            .iter()
            .filter_map(|dependency| self.runtime_signal_for_backend_item(owner, *dependency))
            .collect()
    }

    fn collect_linked_temporal_helpers(
        &self,
        binding: &crate::hir_adapter::HirSignalBinding,
        item: &aivi_backend::Item,
    ) -> Vec<LinkedTemporalHelper> {
        let mut helpers = Vec::new();
        let mut inputs = binding.temporal_helper_inputs().iter().copied();
        for (pipeline_position, &pipeline_id) in item.pipelines.iter().enumerate() {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for (stage_offset, stage) in pipeline.stages.iter().enumerate() {
                let needs_helper = matches!(
                    stage.kind,
                    BackendStageKind::Temporal(BackendTemporalStage::Delay { .. })
                        | BackendStageKind::Temporal(BackendTemporalStage::Burst { .. })
                );
                if !needs_helper {
                    continue;
                }
                let Some(input) = inputs.next() else {
                    return Vec::new();
                };
                let Some(dependency_index) = binding
                    .dependencies()
                    .iter()
                    .position(|&signal| signal == input.as_signal())
                else {
                    return Vec::new();
                };
                helpers.push(LinkedTemporalHelper {
                    input,
                    dependency_index,
                    pipeline: pipeline_id,
                    pipeline_position,
                    stage_index: stage.index,
                    stage_offset,
                });
            }
        }
        helpers
    }

    fn supported_body_backed_derived_signal_pipelines(&self, item: &aivi_backend::Item) -> bool {
        item.body.is_some()
            && item.pipelines.iter().copied().all(|pipeline_id| {
                let pipeline = &self.backend.pipelines()[pipeline_id];
                // Recurrence pipelines require scheduler wakeup infrastructure
                // that is not yet wired; keep them as an explicit boundary.
                pipeline.recurrence.is_none()
                    && pipeline.stages.iter().all(|stage| {
                        matches!(
                            stage.kind,
                            BackendStageKind::TruthyFalsy(_)
                                | BackendStageKind::Gate(BackendGateStage::SignalFilter { .. })
                                | BackendStageKind::Temporal(_)
                                | BackendStageKind::Fanout(_)
                        )
                    })
            })
    }

    fn supported_body_backed_reactive_signal_pipelines(&self, item: &aivi_backend::Item) -> bool {
        item.body.is_some()
            && item.pipelines.iter().copied().all(|pipeline_id| {
                let pipeline = &self.backend.pipelines()[pipeline_id];
                pipeline.recurrence.is_none()
                    && pipeline.stages.iter().all(|stage| {
                        matches!(
                            stage.kind,
                            BackendStageKind::TruthyFalsy(_)
                                | BackendStageKind::Gate(BackendGateStage::SignalFilter { .. })
                                | BackendStageKind::Fanout(_)
                        )
                    })
            })
    }
}

fn same_items(left: &[BackendItemId], right: &[BackendItemId]) -> bool {
    left.len() == right.len()
        && left.iter().copied().collect::<BTreeSet<_>>()
            == right.iter().copied().collect::<BTreeSet<_>>()
}

fn active_when_value(
    instance: SourceInstanceId,
    value: Option<&RuntimeValue>,
) -> Result<bool, BackendRuntimeError> {
    match value {
        None => Ok(false),
        Some(RuntimeValue::Bool(value)) => Ok(*value),
        Some(RuntimeValue::Signal(value)) => match value.as_ref() {
            RuntimeValue::Bool(value) => Ok(*value),
            other => Err(BackendRuntimeError::InvalidActiveWhenValue {
                instance,
                value: other.clone(),
            }),
        },
        Some(other) => Err(BackendRuntimeError::InvalidActiveWhenValue {
            instance,
            value: other.clone(),
        }),
    }
}
