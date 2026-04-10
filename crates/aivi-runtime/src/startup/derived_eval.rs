struct LinkedDerivedEvaluator<'a> {
    backend: &'a BackendProgram,
    signal_items_by_handle: &'a BTreeMap<SignalHandle, BackendItemId>,
    derived_signals: &'a BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    native_kernel_plans: &'a mut BTreeMap<NativePlanCacheKey, NativeKernelPlan>,
    reactive_signals: &'a BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: &'a BTreeMap<crate::ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: &'a BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    committed_signals: &'a BTreeMap<BackendItemId, RuntimeValue>,
    temporal_states: &'a mut BTreeMap<TemporalStageKey, RuntimeValue>,
    pending_temporal_schedules: &'a mut Vec<PendingTemporalSchedule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReactivePipelineContext {
    Seed,
    Body(ReactiveClauseHandle),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum NativePlanCacheKey {
    Derived(DerivedHandle),
    ReactiveSeed(SignalHandle),
    ReactiveGuard(ReactiveClauseHandle),
    ReactiveBody(ReactiveClauseHandle),
}

impl TryDerivedNodeEvaluator<RuntimeValue> for LinkedDerivedEvaluator<'_> {
    type Error = BackendRuntimeError;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        // Check recurrence signals first.
        if let Some(binding) = self.linked_recurrence_signals.get(&signal) {
            return self.try_evaluate_recurrence(signal, binding, inputs);
        }

        let binding = self
            .derived_signals
            .get(&signal)
            .cloned()
            .ok_or(BackendRuntimeError::UnknownDerivedSignal { signal })?;
        let expected_inputs = binding.dependency_items.len()
            + usize::from(binding.source_input.is_some())
            + binding.temporal_helpers.len();
        if inputs.len() != expected_inputs {
            return Err(BackendRuntimeError::DerivedDependencyArityMismatch {
                signal,
                expected: expected_inputs,
                found: inputs.len(),
            });
        }

        // If no upstream dependency changed this tick and the signal already has a committed
        // value, the output of this pure function is guaranteed to be the same — skip the
        // (potentially expensive) kernel re-evaluation.
        let any_updated = (0..expected_inputs).any(|i| inputs.updated(i));
        if !any_updated && self.committed_signals.contains_key(&binding.backend_item) {
            return Ok(DerivedSignalUpdate::Unchanged);
        }

        let mut globals = self.committed_signals.clone();
        globals.remove(&binding.backend_item);
        let mut dependency_environment = Vec::with_capacity(binding.dependency_items.len());
        for (index, dependency) in binding.dependency_items.iter().copied().enumerate() {
            let Some(value) = inputs.value(index) else {
                return Ok(DerivedSignalUpdate::Clear);
            };
            let signal_value = RuntimeValue::Signal(Box::new(value.clone()));
            let layout = binding
                .dependency_layouts
                .get(index)
                .copied()
                .expect("linked derived signal should preserve dependency layouts");
            dependency_environment.push(stage_subject_value(self.backend, layout, value));
            globals.insert(dependency, signal_value);
        }

        let binding_item = binding.item;
        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();
        if let Some(helper) = self.updated_temporal_helper(&binding, &inputs) {
            let Some(value) = inputs.value(helper.dependency_index).cloned() else {
                return Ok(self.suppressed_derived_update(binding.backend_item));
            };
            return self.apply_pipelines_from(
                signal,
                binding_item,
                binding.pipeline_ids.as_ref(),
                Some(TemporalResumePoint {
                    pipeline_position: helper.pipeline_position,
                    stage_offset: helper.stage_offset + 1,
                }),
                value,
                &globals,
                &mut *engine,
            );
        }

        let value =
            self.evaluate_derived_value(signal, &binding, &dependency_environment, &globals)?;

        self.apply_pipelines_from(
            signal,
            binding_item,
            binding.pipeline_ids.as_ref(),
            None,
            value,
            &globals,
            &mut *engine,
        )
    }

    fn try_evaluate_reactive_seed(
        &mut self,
        signal: SignalHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        let binding = self
            .reactive_signals
            .get(&signal)
            .cloned()
            .ok_or(BackendRuntimeError::UnknownReactiveSignal { signal })?;
        if !binding.has_seed_body || !inputs.all_present() {
            return Ok(DerivedSignalUpdate::Clear);
        }

        let globals = self.build_seed_globals(binding.backend_item, &inputs);
        let mut dependency_environment = Vec::with_capacity(binding.dependency_items.len());
        for (index, layout) in binding.dependency_layouts.iter().copied().enumerate() {
            let Some(value) = inputs.value(index) else {
                return Ok(DerivedSignalUpdate::Clear);
            };
            dependency_environment.push(stage_subject_value(self.backend, layout, value));
        }
        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();
        let value =
            self.evaluate_reactive_seed_value(signal, &binding, &dependency_environment, &globals)?;
        let binding_item = binding.item;
        let binding_pipeline_ids = binding.pipeline_ids.clone();
        self.apply_reactive_pipelines(
            signal,
            binding_item,
            ReactivePipelineContext::Seed,
            binding_pipeline_ids.as_ref(),
            value,
            &globals,
            &mut *engine,
        )
    }

    fn try_evaluate_reactive_guard(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<bool, Self::Error> {
        if !inputs.all_present() {
            return Ok(false);
        }
        let binding = self.reactive_clause(signal, clause)?.clone();
        if binding.body_mode == hir::ReactiveUpdateBodyMode::OptionalPayload {
            return Ok(true);
        }
        let Some(globals) = self.build_fragment_globals(&binding.compiled_guard, &inputs) else {
            return Ok(false);
        };
        let value = self.evaluate_reactive_guard_value(signal, clause, &binding, &globals)?;
        match value {
            RuntimeValue::Bool(value) => Ok(value),
            RuntimeValue::Signal(inner) => match *inner {
                RuntimeValue::Bool(value) => Ok(value),
                other => Err(BackendRuntimeError::ReactiveGuardReturnedNonBool {
                    signal,
                    clause,
                    item: binding.owner,
                    value: RuntimeValue::Signal(Box::new(other)),
                }),
            },
            other => Err(BackendRuntimeError::ReactiveGuardReturnedNonBool {
                signal,
                clause,
                item: binding.owner,
                value: other,
            }),
        }
    }

    fn try_evaluate_reactive_body(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        if !inputs.all_present() {
            return Ok(DerivedSignalUpdate::Clear);
        }
        let signal_binding = self
            .reactive_signals
            .get(&signal)
            .cloned()
            .ok_or(BackendRuntimeError::UnknownReactiveSignal { signal })?;
        let clause_binding = self.reactive_clause(signal, clause)?.clone();
        let Some(fragment_globals) =
            self.build_fragment_globals(&clause_binding.compiled_body, &inputs)
        else {
            return Ok(DerivedSignalUpdate::Clear);
        };
        // Clone the Arc so that `fragment_engine`'s lifetime is not tied to the
        // `clause_binding` borrow of `self`, allowing `self.apply_reactive_pipelines`
        // to take a mutable borrow later.
        let fragment_backend = Arc::clone(&clause_binding.compiled_body.backend);
        let clause_owner = clause_binding.owner;
        let body_mode = clause_binding.body_mode;
        let clause_pipeline_ids = clause_binding.pipeline_ids.clone();
        let signal_backend_item = signal_binding.backend_item;
        // `clause_binding` and `signal_binding` borrows of `self` end here (NLL).
        let value = self.evaluate_reactive_body_value(
            signal,
            clause,
            &clause_binding,
            fragment_backend.as_ref(),
            &fragment_globals,
        )?;
        let value = match body_mode {
            hir::ReactiveUpdateBodyMode::Payload => value,
            hir::ReactiveUpdateBodyMode::OptionalPayload => match value {
                RuntimeValue::OptionSome(value) => *value,
                RuntimeValue::OptionNone => return Ok(DerivedSignalUpdate::Unchanged),
                RuntimeValue::Signal(inner) => match *inner {
                    RuntimeValue::OptionSome(value) => *value,
                    RuntimeValue::OptionNone => return Ok(DerivedSignalUpdate::Unchanged),
                    other => {
                        return Err(BackendRuntimeError::ReactiveBodyReturnedNonOption {
                            signal,
                            clause,
                            item: clause_owner,
                            value: RuntimeValue::Signal(Box::new(other)),
                        });
                    }
                },
                other => {
                    return Err(BackendRuntimeError::ReactiveBodyReturnedNonOption {
                        signal,
                        clause,
                        item: clause_owner,
                        value: other,
                    });
                }
            },
        };
        let globals = self.build_signal_globals(signal_backend_item, &inputs);
        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();
        self.apply_reactive_pipelines(
            signal,
            clause_owner,
            ReactivePipelineContext::Body(clause),
            clause_pipeline_ids.as_ref(),
            value,
            &globals,
            &mut *engine,
        )
    }
}

impl LinkedDerivedEvaluator<'_> {
    fn reactive_clause(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
    ) -> Result<&LinkedReactiveClause, BackendRuntimeError> {
        let binding = self
            .reactive_clauses
            .get(&clause)
            .ok_or(BackendRuntimeError::UnknownReactiveClause { signal, clause })?;
        if binding.target != signal {
            return Err(BackendRuntimeError::UnknownReactiveClause { signal, clause });
        }
        Ok(binding)
    }

    fn build_signal_globals(
        &self,
        target_item: BackendItemId,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> BTreeMap<BackendItemId, RuntimeValue> {
        let mut globals = self.committed_signals.clone();
        globals.remove(&target_item);
        for index in 0..inputs.len() {
            let Some(signal) = inputs.signal(index) else {
                continue;
            };
            let Some(&dependency) = self.signal_items_by_handle.get(&signal) else {
                continue;
            };
            match inputs.value(index) {
                Some(value) => {
                    globals.insert(dependency, signal_global_value(value));
                }
                None => {
                    globals.remove(&dependency);
                }
            }
        }
        globals
    }

    fn build_seed_globals(
        &self,
        target_item: BackendItemId,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> BTreeMap<BackendItemId, RuntimeValue> {
        let mut globals = self.committed_signals.clone();
        globals.remove(&target_item);
        for index in 0..inputs.len() {
            let Some(signal) = inputs.signal(index) else {
                continue;
            };
            let Some(&dependency) = self.signal_items_by_handle.get(&signal) else {
                continue;
            };
            match inputs.value(index) {
                Some(value) => {
                    globals.insert(dependency, value.clone());
                }
                None => {
                    globals.remove(&dependency);
                }
            }
        }
        globals
    }

    fn build_fragment_globals(
        &self,
        fragment: &HirCompiledRuntimeExpr,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> Option<BTreeMap<BackendItemId, RuntimeValue>> {
        let mut globals = BTreeMap::new();
        for required in fragment.required_signals.iter() {
            let value = self.fragment_signal_value_for_inputs(inputs, required.signal)?;
            globals.insert(required.backend_item, value);
        }
        Some(globals)
    }

    fn fragment_signal_value_for_inputs(
        &self,
        inputs: &DependencyValues<'_, RuntimeValue>,
        signal: SignalHandle,
    ) -> Option<RuntimeValue> {
        if inputs.contains_signal(signal) {
            return inputs.value_for(signal).map(signal_global_value);
        }
        let backend_item = self.signal_items_by_handle.get(&signal)?;
        self.committed_signals
            .get(backend_item)
            .map(signal_global_value)
    }

    fn updated_temporal_helper<'b>(
        &self,
        binding: &'b LinkedDerivedSignal,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> Option<&'b LinkedTemporalHelper> {
        binding
            .temporal_helpers
            .iter()
            .rev()
            .find(|helper| inputs.updated(helper.dependency_index))
    }

    fn suppressed_derived_update(
        &self,
        backend_item: BackendItemId,
    ) -> DerivedSignalUpdate<RuntimeValue> {
        if self.committed_signals.contains_key(&backend_item) {
            DerivedSignalUpdate::Unchanged
        } else {
            DerivedSignalUpdate::Clear
        }
    }

    fn evaluate_derived_value(
        &mut self,
        signal: DerivedHandle,
        binding: &LinkedDerivedSignal,
        dependency_environment: &[RuntimeValue],
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        match &binding.eval_lane {
            LinkedEvalLane::Native(native) => {
                let plan = self
                    .native_plan(NativePlanCacheKey::Derived(signal), self.backend, native)
                    .ok_or(BackendRuntimeError::MissingNativeDerivedPlan {
                        signal,
                        item: binding.item,
                        kernel: native.kernel,
                    })?;
                match plan.execute(None, dependency_environment, globals) {
                    Ok(value) => Ok(value),
                    Err(NativeKernelExecutionError::FallbackRequired) => {
                        self.evaluate_fallback_derived_value(
                            signal,
                            binding,
                            dependency_environment,
                            globals,
                        )
                    }
                    Err(NativeKernelExecutionError::Evaluation(error)) => {
                        Err(self.derived_eval_error(signal, binding.item, error))
                    }
                }
            }
            LinkedEvalLane::Fallback => {
                self.evaluate_fallback_derived_value(signal, binding, dependency_environment, globals)
            }
        }
    }

    fn evaluate_reactive_seed_value(
        &mut self,
        signal: SignalHandle,
        binding: &LinkedReactiveSignal,
        dependency_environment: &[RuntimeValue],
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        match &binding.seed_eval_lane {
            LinkedEvalLane::Native(native) => {
                let plan = self
                    .native_plan(NativePlanCacheKey::ReactiveSeed(signal), self.backend, native)
                    .ok_or(BackendRuntimeError::MissingNativeReactiveSeedPlan {
                        signal,
                        item: binding.item,
                        kernel: native.kernel,
                    })?;
                match plan.execute(None, dependency_environment, globals) {
                    Ok(value) => Ok(value),
                    Err(NativeKernelExecutionError::FallbackRequired) => self
                        .evaluate_fallback_reactive_seed_value(
                            signal,
                            binding,
                            dependency_environment,
                            globals,
                        ),
                    Err(NativeKernelExecutionError::Evaluation(error)) => {
                        Err(self.reactive_seed_eval_error(signal, binding.item, error))
                    }
                }
            }
            LinkedEvalLane::Fallback => self.evaluate_fallback_reactive_seed_value(
                signal,
                binding,
                dependency_environment,
                globals,
            ),
        }
    }

    fn evaluate_fallback_reactive_seed_value(
        &self,
        signal: SignalHandle,
        binding: &LinkedReactiveSignal,
        dependency_environment: &[RuntimeValue],
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();
        if let Some(body_kernel) = binding.body_kernel {
            engine
                .evaluate_signal_body_kernel(body_kernel, dependency_environment, globals)
                .map_err(|error| self.reactive_seed_eval_error(signal, binding.item, error))
        } else {
            engine
                .evaluate_item(binding.backend_item, globals)
                .map_err(|error| self.reactive_seed_eval_error(signal, binding.item, error))
        }
    }

    fn evaluate_reactive_guard_value(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        binding: &LinkedReactiveClause,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        match &binding.guard_eval_lane {
            LinkedEvalLane::Native(native) => {
                let plan = self
                    .native_plan(
                        NativePlanCacheKey::ReactiveGuard(clause),
                        binding.compiled_guard.backend.as_ref(),
                        native,
                    )
                    .ok_or(BackendRuntimeError::MissingNativeReactiveGuardPlan {
                        signal,
                        clause,
                        item: binding.owner,
                        kernel: native.kernel,
                    })?;
                match plan.execute(None, &[], globals) {
                    Ok(value) => Ok(value),
                    Err(NativeKernelExecutionError::FallbackRequired) => {
                        self.evaluate_fallback_reactive_fragment_value(
                            binding.compiled_guard.backend.as_ref(),
                            binding.compiled_guard.entry_item,
                            |error| self.reactive_guard_eval_error(signal, clause, binding.owner, error),
                            globals,
                        )
                    }
                    Err(NativeKernelExecutionError::Evaluation(error)) => Err(
                        self.reactive_guard_eval_error(signal, clause, binding.owner, error),
                    ),
                }
            }
            LinkedEvalLane::Fallback => self.evaluate_fallback_reactive_fragment_value(
                binding.compiled_guard.backend.as_ref(),
                binding.compiled_guard.entry_item,
                |error| self.reactive_guard_eval_error(signal, clause, binding.owner, error),
                globals,
            ),
        }
    }

    fn evaluate_reactive_body_value(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        binding: &LinkedReactiveClause,
        backend: &BackendProgram,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        match &binding.body_eval_lane {
            LinkedEvalLane::Native(native) => {
                let plan = self
                    .native_plan(NativePlanCacheKey::ReactiveBody(clause), backend, native)
                    .ok_or(BackendRuntimeError::MissingNativeReactiveBodyPlan {
                        signal,
                        clause,
                        item: binding.owner,
                        kernel: native.kernel,
                    })?;
                match plan.execute(None, &[], globals) {
                    Ok(value) => Ok(value),
                    Err(NativeKernelExecutionError::FallbackRequired) => self
                        .evaluate_fallback_reactive_fragment_value(
                            backend,
                            binding.compiled_body.entry_item,
                            |error| self.reactive_body_eval_error(signal, clause, binding.owner, error),
                            globals,
                        ),
                    Err(NativeKernelExecutionError::Evaluation(error)) => {
                        Err(self.reactive_body_eval_error(signal, clause, binding.owner, error))
                    }
                }
            }
            LinkedEvalLane::Fallback => self.evaluate_fallback_reactive_fragment_value(
                backend,
                binding.compiled_body.entry_item,
                |error| self.reactive_body_eval_error(signal, clause, binding.owner, error),
                globals,
            ),
        }
    }

    fn evaluate_fallback_derived_value(
        &self,
        signal: DerivedHandle,
        binding: &LinkedDerivedSignal,
        dependency_environment: &[RuntimeValue],
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();
        if let Some(body_kernel) = binding.body_kernel {
            engine
                .evaluate_signal_body_kernel(body_kernel, dependency_environment, globals)
                .map_err(|error| self.derived_eval_error(signal, binding.item, error))
        } else {
            engine
                .evaluate_item(binding.backend_item, globals)
                .map_err(|error| self.derived_eval_error(signal, binding.item, error))
        }
    }

    fn evaluate_fallback_reactive_fragment_value(
        &self,
        backend: &BackendProgram,
        entry_item: BackendItemId,
        map_error: impl FnOnce(EvaluationError) -> BackendRuntimeError,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let mut engine = BackendExecutableProgram::interpreted(backend).create_engine();
        engine.evaluate_item(entry_item, globals).map_err(map_error)
    }

    fn native_plan<'a>(
        &'a mut self,
        key: NativePlanCacheKey,
        backend: &BackendProgram,
        native: &LinkedNativeKernelEval,
    ) -> Option<&'a mut NativeKernelPlan> {
        if let std::collections::btree_map::Entry::Vacant(e) = self.native_kernel_plans.entry(key) {
            let compiled = NativeKernelPlan::compile(backend, native.kernel)?;
            e.insert(compiled);
        }
        self.native_kernel_plans.get_mut(&key)
    }

    fn derived_eval_error(
        &self,
        signal: DerivedHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateDerivedSignal {
            signal,
            item,
            error,
        }
    }

    fn reactive_seed_eval_error(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveSeed {
            signal,
            item,
            error,
        }
    }

    fn reactive_guard_eval_error(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveGuard {
            signal,
            clause,
            item,
            error,
        }
    }

    fn reactive_body_eval_error(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveBody {
            signal,
            clause,
            item,
            error,
        }
    }

    fn reactive_pipeline_eval_error(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        match context {
            ReactivePipelineContext::Seed => self.reactive_seed_eval_error(signal, item, error),
            ReactivePipelineContext::Body(clause) => {
                self.reactive_body_eval_error(signal, clause, item, error)
            }
        }
    }

    fn apply_pipelines_from(
        &mut self,
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline_ids: &[BackendPipelineId],
        mut resume: Option<TemporalResumePoint>,
        mut value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut dyn BackendExecutionEngine,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        let binding = self
            .derived_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownDerivedSignal { signal })?
            .clone();
        for (pipeline_position, &pipeline_id) in pipeline_ids.iter().enumerate() {
            let stage_start = match resume {
                Some(point) if pipeline_position < point.pipeline_position => continue,
                Some(point) if pipeline_position == point.pipeline_position => {
                    resume = None;
                    point.stage_offset
                }
                _ => 0,
            };
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in pipeline.stages.iter().skip(stage_start) {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate,
                        emits_negative_update,
                        ..
                    }) => {
                        let pred = evaluator
                            .evaluate_kernel(*predicate, Some(&value), &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        if matches!(pred, RuntimeValue::Bool(false)) && !emits_negative_update {
                            return Ok(DerivedSignalUpdate::Clear);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {
                        // Carrier metadata only; the body kernel already computed
                        // the branch result, so we pass the value through unchanged.
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Previous { seed, .. }) => {
                        let key = TemporalStageKey {
                            pipeline: pipeline_id,
                            stage_index: stage.index,
                        };
                        let current = value.clone();
                        value = match self.temporal_states.get(&key).cloned() {
                            Some(previous) => previous,
                            None => evaluator
                                .evaluate_kernel(*seed, None, &[], globals)
                                .map_err(|error| self.derived_eval_error(signal, item, error))?,
                        };
                        self.temporal_states.insert(key, current);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::DiffFunction {
                        diff, ..
                    }) => {
                        let key = TemporalStageKey {
                            pipeline: pipeline_id,
                            stage_index: stage.index,
                        };
                        let current = value.clone();
                        let previous = self
                            .temporal_states
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| current.clone());
                        let callable = evaluator
                            .evaluate_kernel(*diff, None, &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        value = evaluator
                            .apply_runtime_callable(
                                *diff,
                                callable,
                                vec![previous, current.clone()],
                                globals,
                            )
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        self.temporal_states.insert(key, current);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::DiffSeed { seed, .. }) => {
                        let key = TemporalStageKey {
                            pipeline: pipeline_id,
                            stage_index: stage.index,
                        };
                        let current = value.clone();
                        let previous = match self.temporal_states.get(&key).cloned() {
                            Some(previous) => previous,
                            None => evaluator
                                .evaluate_kernel(*seed, None, &[], globals)
                                .map_err(|error| self.derived_eval_error(signal, item, error))?,
                        };
                        value = evaluator
                            .subtract_runtime_values(*seed, current.clone(), previous)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        self.temporal_states.insert(key, current);
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Delay {
                        duration, ..
                    }) => {
                        let duration_value = evaluator
                            .evaluate_kernel(*duration, None, &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        let wait = parse_temporal_duration(&duration_value).ok_or_else(|| {
                            BackendRuntimeError::InvalidTemporalDelayDuration {
                                signal,
                                item,
                                pipeline: pipeline_id,
                                stage_index: stage.index,
                                value: duration_value.clone(),
                            }
                        })?;
                        if wait.is_zero() {
                            return Err(BackendRuntimeError::InvalidTemporalDelayDuration {
                                signal,
                                item,
                                pipeline: pipeline_id,
                                stage_index: stage.index,
                                value: duration_value,
                            });
                        }
                        self.pending_temporal_schedules
                            .push(PendingTemporalSchedule {
                                signal,
                                item,
                                key: TemporalStageKey {
                                    pipeline: pipeline_id,
                                    stage_index: stage.index,
                                },
                                value: value.clone(),
                                kind: TemporalWorkerScheduleKind::Delay { wait },
                            });
                        return Ok(self.suppressed_derived_update(binding.backend_item));
                    }
                    BackendStageKind::Temporal(BackendTemporalStage::Burst {
                        every,
                        count,
                        ..
                    }) => {
                        let every_value = evaluator
                            .evaluate_kernel(*every, None, &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        let wait = parse_temporal_duration(&every_value).ok_or_else(|| {
                            BackendRuntimeError::InvalidTemporalBurstInterval {
                                signal,
                                item,
                                pipeline: pipeline_id,
                                stage_index: stage.index,
                                value: every_value.clone(),
                            }
                        })?;
                        let count_value = evaluator
                            .evaluate_kernel(*count, None, &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        let repetitions = parse_temporal_count(&count_value).ok_or_else(|| {
                            BackendRuntimeError::InvalidTemporalBurstCount {
                                signal,
                                item,
                                pipeline: pipeline_id,
                                stage_index: stage.index,
                                value: count_value.clone(),
                            }
                        })?;
                        if wait.is_zero() {
                            return Err(BackendRuntimeError::InvalidTemporalBurstInterval {
                                signal,
                                item,
                                pipeline: pipeline_id,
                                stage_index: stage.index,
                                value: every_value,
                            });
                        }
                        self.pending_temporal_schedules
                            .push(PendingTemporalSchedule {
                                signal,
                                item,
                                key: TemporalStageKey {
                                    pipeline: pipeline_id,
                                    stage_index: stage.index,
                                },
                                value: value.clone(),
                                kind: TemporalWorkerScheduleKind::Burst {
                                    wait,
                                    remaining: repetitions,
                                },
                            });
                        return Ok(self.suppressed_derived_update(binding.backend_item));
                    }
                    BackendStageKind::Fanout(fanout) => {
                        value = self
                            .apply_fanout_stage(signal, item, fanout, value, globals, evaluator)?;
                    }
                    _ => unreachable!(
                        "unsupported pipeline stage kind should have been blocked during linking"
                    ),
                }
            }
        }
        Ok(DerivedSignalUpdate::Value(value))
    }

    fn apply_reactive_pipelines(
        &mut self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        pipeline_ids: &[BackendPipelineId],
        mut value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut dyn BackendExecutionEngine,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        for &pipeline_id in pipeline_ids {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate,
                        emits_negative_update,
                        ..
                    }) => {
                        let pred = evaluator
                            .evaluate_kernel(*predicate, Some(&value), &[], globals)
                            .map_err(|error| {
                                self.reactive_pipeline_eval_error(signal, item, context, error)
                            })?;
                        if matches!(pred, RuntimeValue::Bool(false)) && !emits_negative_update {
                            return Ok(DerivedSignalUpdate::Clear);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {}
                    BackendStageKind::Temporal(_) => unreachable!(
                        "reactive temporal stages should be rejected during runtime linking"
                    ),
                    BackendStageKind::Fanout(fanout) => {
                        value = self.apply_reactive_fanout_stage(
                            signal, item, context, fanout, value, globals, evaluator,
                        )?;
                    }
                    _ => unreachable!(
                        "unsupported pipeline stage kind should have been blocked during linking"
                    ),
                }
            }
        }
        Ok(DerivedSignalUpdate::Value(value))
    }

    fn apply_fanout_stage(
        &self,
        signal: DerivedHandle,
        item: hir::ItemId,
        fanout: &aivi_backend::FanoutStage,
        value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut dyn BackendExecutionEngine,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let current = match value {
            RuntimeValue::Signal(inner) => *inner,
            other => other,
        };
        let RuntimeValue::List(elements) = current else {
            return Ok(current);
        };

        let mut mapped = Vec::with_capacity(elements.len());
        'elements: for element in elements {
            let mapped_value = evaluator
                .evaluate_kernel(fanout.map, Some(&element), &[], globals)
                .map_err(|error| self.derived_eval_error(signal, item, error))?;
            for filter in &fanout.filters {
                let predicate = evaluator
                    .evaluate_kernel(filter.predicate, Some(&mapped_value), &[], globals)
                    .map_err(|error| self.derived_eval_error(signal, item, error))?;
                if !matches!(predicate, RuntimeValue::Bool(true)) {
                    continue 'elements;
                }
            }
            mapped.push(mapped_value);
        }

        let mapped_collection = RuntimeValue::List(mapped);
        match &fanout.join {
            Some(join) => {
                let subject =
                    stage_subject_value(self.backend, join.input_layout, &mapped_collection);
                let joined = evaluator
                    .evaluate_kernel(join.kernel, Some(&subject), &[], globals)
                    .map_err(|error| self.derived_eval_error(signal, item, error))?;
                Ok(unwrap_signal_layout_result(
                    self.backend,
                    join.result_layout,
                    joined,
                ))
            }
            None => Ok(mapped_collection),
        }
    }

    fn apply_reactive_fanout_stage(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        fanout: &aivi_backend::FanoutStage,
        value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut dyn BackendExecutionEngine,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let current = match value {
            RuntimeValue::Signal(inner) => *inner,
            other => other,
        };
        let RuntimeValue::List(elements) = current else {
            return Ok(current);
        };

        let mut mapped = Vec::with_capacity(elements.len());
        'elements: for element in elements {
            let mapped_value = evaluator
                .evaluate_kernel(fanout.map, Some(&element), &[], globals)
                .map_err(|error| self.reactive_pipeline_eval_error(signal, item, context, error))?;
            for filter in &fanout.filters {
                let predicate = evaluator
                    .evaluate_kernel(filter.predicate, Some(&mapped_value), &[], globals)
                    .map_err(|error| {
                        self.reactive_pipeline_eval_error(signal, item, context, error)
                    })?;
                if !matches!(predicate, RuntimeValue::Bool(true)) {
                    continue 'elements;
                }
            }
            mapped.push(mapped_value);
        }

        let mapped_collection = RuntimeValue::List(mapped);
        match &fanout.join {
            Some(join) => {
                let subject =
                    stage_subject_value(self.backend, join.input_layout, &mapped_collection);
                let joined = evaluator
                    .evaluate_kernel(join.kernel, Some(&subject), &[], globals)
                    .map_err(|error| {
                        self.reactive_pipeline_eval_error(signal, item, context, error)
                    })?;
                Ok(unwrap_signal_layout_result(
                    self.backend,
                    join.result_layout,
                    joined,
                ))
            }
            None => Ok(mapped_collection),
        }
    }

    fn try_evaluate_recurrence(
        &self,
        signal: DerivedHandle,
        binding: &LinkedRecurrenceSignal,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        let mut globals = self.committed_signals.clone();
        for (index, &dep_item) in binding.dependency_items.iter().enumerate() {
            if let Some(value) = inputs.value(index) {
                globals.insert(dep_item, RuntimeValue::Signal(Box::new(value.clone())));
            }
        }

        let wakeup_fired = inputs.updated(binding.wakeup_dependency_index);

        let previous = self.committed_signals.get(&binding.backend_item).cloned();

        let mut engine = BackendExecutableProgram::interpreted(self.backend).create_engine();

        if previous.is_none() {
            // First tick: evaluate the seed kernel (no input subject).
            let seed_value = evaluate_kernel_coercing_zero_arity(
                &mut *engine,
                binding.seed_kernel,
                None,
                &globals,
            )
            .map_err(|error| BackendRuntimeError::EvaluateRecurrenceSignal {
                signal,
                item: binding.item,
                error,
            })?;
            return Ok(DerivedSignalUpdate::Value(seed_value));
        }

        if !wakeup_fired {
            // Non-wakeup dependencies (for example a separate direction signal captured
            // by an accumulate step) may change between firings. Preserve the last committed
            // accumulator snapshot until the wakeup dependency actually fires.
            return Ok(DerivedSignalUpdate::Unchanged);
        }

        // Wakeup fired: evaluate step kernel with previous value as subject.
        let prev_value = previous.unwrap();
        let actual_prev = match &prev_value {
            RuntimeValue::Signal(inner) => inner.as_ref(),
            other => other,
        };
        let mut result = actual_prev.clone();
        for &step_kernel in binding.step_kernels.iter() {
            result = engine
                .evaluate_kernel(step_kernel, Some(&result), &[], &globals)
                .map_err(|error| BackendRuntimeError::EvaluateRecurrenceSignal {
                    signal,
                    item: binding.item,
                    error,
                })?;
        }

        Ok(DerivedSignalUpdate::Value(result))
    }
}

/// Evaluate a kernel, applying zero-arity sum constructors when the kernel returns a Callable
/// where a fully-applied Sum value is expected.
///
/// This is needed for seed expressions that are zero-arity sum constructors (e.g., `Direction.Right`).
/// The kernel evaluator always returns `Callable(SumConstructor)` for constructor references, even
/// when the constructor has no fields. This helper detects that case and applies the constructor to
/// produce the expected `Sum` value.
fn evaluate_kernel_coercing_zero_arity(
    evaluator: &mut dyn BackendExecutionEngine,
    kernel_id: KernelId,
    input_subject: Option<&RuntimeValue>,
    globals: &std::collections::BTreeMap<BackendItemId, RuntimeValue>,
) -> Result<RuntimeValue, EvaluationError> {
    match evaluator.evaluate_kernel(kernel_id, input_subject, &[], globals) {
        Ok(value) => Ok(value),
        Err(EvaluationError::KernelResultLayoutMismatch {
            expected, found, ..
        }) => {
            // If the value is a zero-arity sum constructor callable, apply it to get
            // the actual Sum value.
            if let RuntimeValue::Callable(RuntimeCallable::SumConstructor {
                ref handle,
                ref bound_arguments,
            }) = found
                && handle.field_count == 0 && bound_arguments.is_empty() {
                    return Ok(RuntimeValue::Sum(RuntimeSumValue {
                        item: handle.item,
                        type_name: handle.type_name.clone(),
                        variant_name: handle.variant_name.clone(),
                        fields: Vec::new(),
                    }));
                }
            Err(EvaluationError::KernelResultLayoutMismatch {
                kernel: kernel_id,
                expected,
                found,
            })
        }
        Err(other) => Err(other),
    }
}
