use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};

use aivi_ffi_call::{
    AbiValue, AllocationArena, decode_len_prefixed_bytes, decode_marshaled_map,
    decode_marshaled_sequence, encode_len_prefixed_bytes, encode_marshaled_map,
    encode_marshaled_sequence, read_bigint_constant_bytes, read_decimal_constant_bytes,
    read_marshaled_field, with_active_arena,
};

use crate::{
    AbiPassMode, BackendExecutionEngine, BackendExecutionEngineKind, BackendExecutionOptions,
    EvalFrame, EvaluationCallProfile, EvaluationError, ItemId, KernelEvaluationProfile,
    KernelEvaluator, KernelExprId, KernelFingerprint, KernelId, LayoutId, LayoutKind,
    PrimitiveType, Program, RuntimeBigInt, RuntimeCallable, RuntimeDecimal, RuntimeFloat,
    RuntimeMap, RuntimeMapEntry, RuntimeRecordField, RuntimeValue, TASK_COMPOSITION_KERNEL_ID,
    TaskFunctionApplier,
    cache::compile_kernel_jit_cached,
    codegen::CompiledJitKernel,
    compute_kernel_fingerprint,
    program::ItemKind,
    runtime::{coerce_runtime_value, strip_signal},
};

pub(crate) struct LazyJitExecutionEngine<'a> {
    program: &'a Program,
    fallback: KernelEvaluator<'a>,
    last_kernel_call: Option<LastKernelCall>,
    item_cache: BTreeMap<ItemId, RuntimeValue>,
    item_stack: BTreeSet<ItemId>,
    eval_trace: Vec<EvalFrame>,
    kernel_plans: BTreeMap<KernelFingerprint, CachedKernelPlan>,
    jit_profile: Option<KernelEvaluationProfile>,
    combined_profile: Option<KernelEvaluationProfile>,
}

impl<'a> LazyJitExecutionEngine<'a> {
    pub(crate) fn new(program: &'a Program, options: BackendExecutionOptions) -> Self {
        Self::with_profile(program, options, false)
    }

    pub(crate) fn new_profiled(program: &'a Program, options: BackendExecutionOptions) -> Self {
        Self::with_profile(program, options, true)
    }

    fn with_profile(
        program: &'a Program,
        options: BackendExecutionOptions,
        profiled: bool,
    ) -> Self {
        let fallback = if profiled {
            KernelEvaluator::new_profiled(program)
        } else {
            KernelEvaluator::new(program)
        };
        let mut engine = Self {
            program,
            fallback,
            last_kernel_call: None,
            item_cache: BTreeMap::new(),
            item_stack: BTreeSet::new(),
            eval_trace: Vec::new(),
            kernel_plans: BTreeMap::new(),
            jit_profile: profiled.then(KernelEvaluationProfile::default),
            combined_profile: profiled.then(KernelEvaluationProfile::default),
        };
        if options.eagerly_compile_signals {
            engine.prepare_signal_body_plans();
        }
        engine.refresh_profile();
        engine
    }

    fn refresh_profile(&mut self) {
        let Some(jit_profile) = self.jit_profile.as_ref() else {
            self.combined_profile = None;
            return;
        };
        let mut combined = self.fallback.profile_snapshot().unwrap_or_default();
        combined.merge_from(jit_profile);
        self.combined_profile = Some(combined);
    }

    fn record_kernel_profile(&mut self, kernel: KernelId, elapsed: Duration, cache_hit: bool) {
        if let Some(profile) = &mut self.jit_profile {
            record_call(
                profile.kernels.entry(kernel).or_default(),
                elapsed,
                cache_hit,
            );
            self.refresh_profile();
        }
    }

    fn record_item_profile(&mut self, item: ItemId, elapsed: Duration, cache_hit: bool) {
        if let Some(profile) = &mut self.jit_profile {
            record_call(profile.items.entry(item).or_default(), elapsed, cache_hit);
            self.refresh_profile();
        }
    }

    fn prepare_kernel_plan(&mut self, kernel_id: KernelId) -> KernelFingerprint {
        let fingerprint = compute_kernel_fingerprint(self.program, kernel_id);
        self.kernel_plans
            .entry(fingerprint)
            .or_insert_with(|| CachedKernelPlan::build(self.program, kernel_id));
        fingerprint
    }

    fn prepare_signal_body_plans(&mut self) {
        let kernels = self
            .program
            .items()
            .iter()
            .filter_map(|(_, item)| match &item.kind {
                ItemKind::Signal(signal) => signal.body_kernel,
                _ => None,
            })
            .collect::<Vec<_>>();
        for kernel_id in kernels {
            self.prepare_kernel_plan(kernel_id);
        }
    }
}

impl BackendExecutionEngine for LazyJitExecutionEngine<'_> {
    fn kind(&self) -> BackendExecutionEngineKind {
        BackendExecutionEngineKind::Jit
    }

    fn program(&self) -> &Program {
        self.program
    }

    fn profile(&self) -> Option<&KernelEvaluationProfile> {
        self.combined_profile.as_ref()
    }

    fn profile_snapshot(&self) -> Option<KernelEvaluationProfile> {
        self.combined_profile.clone()
    }

    fn eval_trace(&self) -> &[EvalFrame] {
        &self.eval_trace
    }

    fn evaluate_kernel(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let started_at = self.jit_profile.as_ref().map(|_| Instant::now());
        let kernel = self
            .program
            .kernels()
            .get(kernel_id)
            .cloned()
            .ok_or(EvaluationError::UnknownKernel { kernel: kernel_id })?;
        if let Some((cached_result, cached_layout)) =
            self.last_kernel_call.as_ref().and_then(|last| {
                (last.kernel_id == kernel_id
                    && last.input_subject.as_ref() == input_subject
                    && last.environment.as_ref() == environment)
                    .then(|| (last.result.clone(), last.result_layout))
            })
        {
            self.record_kernel_profile(
                kernel_id,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                true,
            );
            if cached_layout != kernel.result_layout {
                return Err(EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: kernel.result_layout,
                    found: cached_result,
                });
            }
            return Ok(cached_result);
        }

        let fingerprint = self.prepare_kernel_plan(kernel_id);
        let execution = {
            let plan = self
                .kernel_plans
                .get_mut(&fingerprint)
                .expect("prepared plan should remain cached");
            match plan {
                CachedKernelPlan::Compiled(compiled) => {
                    validate_compiled_inputs(compiled, input_subject, environment)?;
                    execute_compiled_kernel(
                        kernel_id,
                        compiled,
                        input_subject,
                        environment,
                        globals,
                    )
                }
                CachedKernelPlan::Fallback => Err(CompiledKernelFailure::Fallback),
            }
        };

        let result = match execution {
            Ok(result) => {
                self.record_kernel_profile(
                    kernel_id,
                    started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                    false,
                );
                result
            }
            Err(CompiledKernelFailure::Evaluation(error)) => return Err(error),
            Err(CompiledKernelFailure::Fallback) => {
                self.kernel_plans
                    .insert(fingerprint, CachedKernelPlan::Fallback);
                let result = self.fallback.evaluate_kernel(
                    kernel_id,
                    input_subject,
                    environment,
                    globals,
                )?;
                self.refresh_profile();
                result
            }
        };

        self.last_kernel_call = Some(LastKernelCall {
            kernel_id,
            input_subject: input_subject.cloned(),
            environment: environment.to_vec().into_boxed_slice(),
            result: result.clone(),
            result_layout: kernel.result_layout,
        });
        Ok(result)
    }

    fn evaluate_signal_body_kernel(
        &mut self,
        kernel_id: KernelId,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let started_at = self.jit_profile.as_ref().map(|_| Instant::now());
        let kernel = self
            .program
            .kernels()
            .get(kernel_id)
            .cloned()
            .ok_or(EvaluationError::UnknownKernel { kernel: kernel_id })?;
        if let Some((cached_result, cached_layout)) =
            self.last_kernel_call.as_ref().and_then(|last| {
                (last.kernel_id == kernel_id
                    && last.input_subject.is_none()
                    && last.environment.as_ref() == environment)
                    .then(|| (last.result.clone(), last.result_layout))
            })
        {
            self.record_kernel_profile(
                kernel_id,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                true,
            );
            if cached_layout != kernel.result_layout {
                return Err(EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: kernel.result_layout,
                    found: cached_result,
                });
            }
            return Ok(cached_result);
        }

        let fingerprint = self.prepare_kernel_plan(kernel_id);
        let execution = {
            let plan = self
                .kernel_plans
                .get_mut(&fingerprint)
                .expect("prepared plan should remain cached");
            match plan {
                CachedKernelPlan::Compiled(compiled) => {
                    validate_compiled_inputs(compiled, None, environment)?;
                    execute_compiled_kernel(kernel_id, compiled, None, environment, globals)
                }
                CachedKernelPlan::Fallback => Err(CompiledKernelFailure::Fallback),
            }
        };

        let raw_result = match execution {
            Ok(result) => {
                self.record_kernel_profile(
                    kernel_id,
                    started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                    false,
                );
                result
            }
            Err(CompiledKernelFailure::Evaluation(error)) => return Err(error),
            Err(CompiledKernelFailure::Fallback) => {
                self.kernel_plans
                    .insert(fingerprint, CachedKernelPlan::Fallback);
                let result =
                    self.fallback
                        .evaluate_signal_body_kernel(kernel_id, environment, globals)?;
                self.refresh_profile();
                result
            }
        };
        let result = crate::runtime::normalize_signal_kernel_result(
            self.program,
            kernel_id,
            raw_result,
            kernel.result_layout,
        )?;
        self.last_kernel_call = Some(LastKernelCall {
            kernel_id,
            input_subject: None,
            environment: environment.to_vec().into_boxed_slice(),
            result: result.clone(),
            result_layout: kernel.result_layout,
        });
        Ok(result)
    }

    fn apply_runtime_callable(
        &mut self,
        kernel_id: KernelId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let callee = strip_signal(callee);
        let RuntimeValue::Callable(callable) = callee else {
            return Err(EvaluationError::InvalidCallee {
                kernel: kernel_id,
                expr: KernelExprId::from_raw(0),
                found: callee,
            });
        };
        match callable {
            RuntimeCallable::ItemBody {
                item,
                kernel,
                parameters,
                mut bound_arguments,
            } => {
                let mut remaining_arguments = Vec::new();
                for argument in arguments {
                    if let Some(expected) = parameters.get(bound_arguments.len()).copied() {
                        let argument = coerce_runtime_value(self.program, argument, expected)
                            .unwrap_or_else(|value| value);
                        bound_arguments.push(argument);
                    } else {
                        remaining_arguments.push(argument);
                    }
                }
                if bound_arguments.len() < parameters.len() {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::ItemBody {
                        item,
                        kernel,
                        parameters,
                        bound_arguments,
                    }));
                }
                let mut remaining = bound_arguments.split_off(parameters.len());
                remaining.extend(remaining_arguments);
                let result = self.evaluate_kernel(kernel, None, &bound_arguments, globals)?;
                if remaining.is_empty() {
                    Ok(result)
                } else {
                    self.apply_runtime_callable(kernel_id, result, remaining, globals)
                }
            }
            other => {
                let result = self.fallback.apply_runtime_callable(
                    kernel_id,
                    RuntimeValue::Callable(other),
                    arguments,
                    globals,
                );
                self.refresh_profile();
                result
            }
        }
    }

    fn subtract_runtime_values(
        &self,
        kernel_id: KernelId,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        self.fallback
            .subtract_runtime_values(kernel_id, left, right)
    }

    fn evaluate_item(
        &mut self,
        item: ItemId,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        if let Some(value) = globals.get(&item) {
            return Ok(value.clone());
        }
        let started_at = self.jit_profile.as_ref().map(|_| Instant::now());
        if let Some(value) = self.item_cache.get(&item).cloned() {
            self.record_item_profile(
                item,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                true,
            );
            return Ok(value);
        }
        let item_decl = self
            .program
            .items()
            .get(item)
            .cloned()
            .ok_or(EvaluationError::UnknownItem { item })?;
        let kernel = item_decl
            .body
            .ok_or(EvaluationError::MissingItemBody { item })?;
        if !item_decl.parameters.is_empty() {
            return Ok(RuntimeValue::Callable(RuntimeCallable::ItemBody {
                item,
                kernel,
                parameters: item_decl.parameters.clone(),
                bound_arguments: Vec::new(),
            }));
        }
        if matches!(item_decl.kind, ItemKind::Signal(_)) {
            let result = self.fallback.evaluate_item(item, globals);
            self.refresh_profile();
            return result;
        }
        if !self.item_stack.insert(item) {
            self.record_item_profile(
                item,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                false,
            );
            return Err(EvaluationError::RecursiveItemEvaluation { item });
        }
        self.eval_trace.push(EvalFrame { item, kernel });
        let result = self.evaluate_kernel(kernel, None, &[], globals);
        self.item_stack.remove(&item);
        let result = match result {
            Ok(value) => {
                self.eval_trace.pop();
                value
            }
            Err(error) => {
                self.record_item_profile(
                    item,
                    started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                    false,
                );
                return Err(error);
            }
        };
        self.record_item_profile(
            item,
            started_at.map_or(Duration::ZERO, |started| started.elapsed()),
            false,
        );
        self.item_cache.insert(item, result.clone());
        Ok(result)
    }
}

impl TaskFunctionApplier for LazyJitExecutionEngine<'_> {
    fn apply_task_function(
        &mut self,
        function: RuntimeValue,
        args: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        self.apply_runtime_callable(TASK_COMPOSITION_KERNEL_ID, function, args, globals)
    }
}

#[cfg(test)]
mod tests {
    use super::LazyJitExecutionEngine;
    use crate::{
        BackendExecutionOptions, ItemId, ItemKind, Program, compute_kernel_fingerprint,
        lower_module as lower_backend_module, validate_program,
    };
    use aivi_base::SourceDatabase;
    use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
    use aivi_lambda::{
        lower_module as lower_lambda_module, validate_module as validate_lambda_module,
    };
    use aivi_syntax::parse_module;

    fn lower_text(path: &str, text: &str) -> Program {
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
        validate_lambda_module(&lambda)
            .expect("typed lambda should validate before backend lowering");

        let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
        validate_program(&backend).expect("backend program should validate");
        backend
    }

    fn find_item(program: &Program, name: &str) -> ItemId {
        program
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("expected backend item `{name}`"))
    }

    #[test]
    fn default_jit_engine_keeps_signal_body_kernels_lazy() {
        let backend = lower_text(
            "jit-signal-body-lazy.aivi",
            r#"
signal base = 1
signal derived = base
"#,
        );
        let derived = find_item(&backend, "derived");
        let body_kernel = match &backend.items()[derived].kind {
            ItemKind::Signal(signal) => signal
                .body_kernel
                .expect("direct signal dependency should lower a body kernel"),
            other => panic!("expected signal item, found {other:?}"),
        };
        let fingerprint = compute_kernel_fingerprint(&backend, body_kernel);

        let engine = LazyJitExecutionEngine::new(&backend, BackendExecutionOptions::default());

        assert!(!engine.kernel_plans.contains_key(&fingerprint));
    }

    #[test]
    fn eager_signal_compilation_prepares_signal_body_kernels() {
        let backend = lower_text(
            "jit-signal-body-eager.aivi",
            r#"
signal base = 1
signal derived = base
"#,
        );
        let derived = find_item(&backend, "derived");
        let body_kernel = match &backend.items()[derived].kind {
            ItemKind::Signal(signal) => signal
                .body_kernel
                .expect("direct signal dependency should lower a body kernel"),
            other => panic!("expected signal item, found {other:?}"),
        };
        let fingerprint = compute_kernel_fingerprint(&backend, body_kernel);

        let engine = LazyJitExecutionEngine::new(
            &backend,
            BackendExecutionOptions {
                eagerly_compile_signals: true,
                prefer_interpreter: false,
            },
        );

        assert!(engine.kernel_plans.contains_key(&fingerprint));
    }
}

fn validate_compiled_inputs(
    plan: &NativeKernelPlan,
    input_subject: Option<&RuntimeValue>,
    environment: &[RuntimeValue],
) -> Result<(), EvaluationError> {
    match (&plan.input_plan, input_subject) {
        (Some(expected), Some(value)) if expected.matches(value) => {}
        (Some(_), Some(value)) => {
            return Err(EvaluationError::KernelInputLayoutMismatch {
                kernel: plan.kernel_id,
                expected: plan
                    .input_layout
                    .expect("compiled subject plan implies subject"),
                found: value.clone(),
            });
        }
        (Some(_), None) => {
            return Err(EvaluationError::MissingInputSubject {
                kernel: plan.kernel_id,
            });
        }
        (None, Some(_)) => {
            return Err(EvaluationError::UnexpectedInputSubject {
                kernel: plan.kernel_id,
            });
        }
        (None, None) => {}
    }
    if environment.len() != plan.environment_plans.len() {
        return Err(EvaluationError::KernelEnvironmentCountMismatch {
            kernel: plan.kernel_id,
            expected: plan.environment_plans.len(),
            found: environment.len(),
        });
    }
    for (index, (expected, value)) in plan
        .environment_plans
        .iter()
        .zip(environment.iter())
        .enumerate()
    {
        if !expected.matches(value) {
            return Err(EvaluationError::KernelEnvironmentLayoutMismatch {
                kernel: plan.kernel_id,
                slot: crate::EnvSlotId::from_raw(index as u32),
                expected: plan.environment_layouts[index],
                found: value.clone(),
            });
        }
    }
    Ok(())
}

fn execute_compiled_kernel(
    kernel_id: KernelId,
    plan: &mut NativeKernelPlan,
    input_subject: Option<&RuntimeValue>,
    environment: &[RuntimeValue],
    globals: &BTreeMap<ItemId, RuntimeValue>,
) -> Result<RuntimeValue, CompiledKernelFailure> {
    let arena = Rc::new(RefCell::new(AllocationArena::new()));
    let mut hints = PackedValueHints::default();
    {
        let mut arena_mut = arena.borrow_mut();
        for (slot, slot_plan) in plan
            .artifact
            .signal_slots
            .iter_mut()
            .zip(plan.signal_slot_plans.iter())
        {
            let value = globals
                .get(&slot.item)
                .ok_or(CompiledKernelFailure::Fallback)?;
            if !slot_plan.write_slot(
                strip_signal_wrappers(value),
                &mut slot.cell,
                &mut arena_mut,
                &mut hints,
            ) {
                return Err(CompiledKernelFailure::Fallback);
            }
        }
        for (slot, slot_plan) in plan
            .artifact
            .imported_item_slots
            .iter_mut()
            .zip(plan.imported_item_slot_plans.iter())
        {
            let value = globals
                .get(&slot.item)
                .ok_or(CompiledKernelFailure::Fallback)?;
            if !slot_plan.write_slot(
                strip_signal_wrappers(value),
                &mut slot.cell,
                &mut arena_mut,
                &mut hints,
            ) {
                return Err(CompiledKernelFailure::Fallback);
            }
        }
    }

    let mut args =
        Vec::with_capacity(plan.environment_plans.len() + usize::from(plan.input_plan.is_some()));
    {
        let mut arena_mut = arena.borrow_mut();
        if let Some(input_plan) = &plan.input_plan {
            let value = input_subject.ok_or(CompiledKernelFailure::Evaluation(
                EvaluationError::MissingInputSubject { kernel: kernel_id },
            ))?;
            let Some(arg) = input_plan.pack_argument(value, &mut arena_mut, &mut hints) else {
                return Err(CompiledKernelFailure::Fallback);
            };
            args.push(arg);
        }
        for (slot_plan, value) in plan.environment_plans.iter().zip(environment.iter()) {
            let Some(arg) = slot_plan.pack_argument(value, &mut arena_mut, &mut hints) else {
                return Err(CompiledKernelFailure::Fallback);
            };
            args.push(arg);
        }
    }

    let call_result = with_active_arena(Rc::clone(&arena), || {
        plan.artifact.caller.call(plan.artifact.function, &args)
    })
    .map_err(|_| CompiledKernelFailure::Fallback)?;
    plan.result_plan
        .unpack_result(call_result, &hints)
        .ok_or(CompiledKernelFailure::Fallback)
}

enum CachedKernelPlan {
    Compiled(NativeKernelPlan),
    Fallback,
}

impl CachedKernelPlan {
    fn build(program: &Program, kernel_id: KernelId) -> Self {
        let Some(compiled) = NativeKernelPlan::compile(program, kernel_id) else {
            return Self::Fallback;
        };
        Self::Compiled(compiled)
    }
}

#[derive(Clone, Debug)]
pub enum NativeKernelExecutionError {
    FallbackRequired,
    Evaluation(EvaluationError),
}

pub struct NativeKernelPlan {
    kernel_id: KernelId,
    input_layout: Option<LayoutId>,
    environment_layouts: Vec<LayoutId>,
    result_layout: LayoutId,
    artifact: CompiledJitKernel,
    input_plan: Option<MarshalPlan>,
    environment_plans: Vec<MarshalPlan>,
    result_plan: MarshalPlan,
    signal_slot_plans: Vec<MarshalPlan>,
    imported_item_slot_plans: Vec<MarshalPlan>,
}

impl NativeKernelPlan {
    pub fn compile(program: &Program, kernel_id: KernelId) -> Option<Self> {
        let kernel = &program.kernels()[kernel_id];
        let input_plan = match kernel.input_subject {
            Some(layout) => Some(MarshalPlan::for_layout(program, layout)?),
            None => None,
        };
        let environment_plans = kernel
            .environment
            .iter()
            .map(|layout| MarshalPlan::for_layout(program, *layout))
            .collect::<Option<Vec<_>>>()?;
        let result_plan = MarshalPlan::for_layout(program, kernel.result_layout)?;
        let artifact = compile_kernel_jit_cached(program, kernel_id).ok()?;
        let signal_slot_plans = artifact
            .signal_slots
            .iter()
            .map(|slot| MarshalPlan::for_layout(program, slot.layout))
            .collect::<Option<Vec<_>>>()?;
        let imported_item_slot_plans = artifact
            .imported_item_slots
            .iter()
            .map(|slot| MarshalPlan::for_layout(program, slot.layout))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            kernel_id,
            input_layout: kernel.input_subject,
            environment_layouts: kernel.environment.clone(),
            result_layout: kernel.result_layout,
            artifact,
            input_plan,
            environment_plans,
            result_plan,
            signal_slot_plans,
            imported_item_slot_plans,
        })
    }

    pub fn kernel_id(&self) -> KernelId {
        self.kernel_id
    }

    pub fn result_layout(&self) -> LayoutId {
        self.result_layout
    }

    pub fn dependency_layouts(&self) -> &[LayoutId] {
        &self.environment_layouts
    }

    pub fn execute(
        &mut self,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, NativeKernelExecutionError> {
        validate_compiled_inputs(self, input_subject, environment)
            .map_err(NativeKernelExecutionError::Evaluation)?;
        match execute_compiled_kernel(self.kernel_id, self, input_subject, environment, globals) {
            Ok(value) => Ok(value),
            Err(CompiledKernelFailure::Fallback) => {
                Err(NativeKernelExecutionError::FallbackRequired)
            }
            Err(CompiledKernelFailure::Evaluation(error)) => {
                Err(NativeKernelExecutionError::Evaluation(error))
            }
        }
    }
}

#[derive(Default)]
struct PackedValueHints(BTreeMap<usize, RuntimeValue>);

impl PackedValueHints {
    fn remember(&mut self, pointer: *const c_void, value: &RuntimeValue) {
        if !pointer.is_null() {
            self.0.insert(pointer as usize, value.clone());
        }
    }

    fn lookup_matching(&self, plan: &MarshalPlan, pointer: *const c_void) -> Option<RuntimeValue> {
        let value = self.0.get(&(pointer as usize))?.clone();
        plan.matches(&value).then_some(value)
    }
}

#[derive(Clone, Copy)]
enum ScalarOptionKind {
    Int,
    Float,
    Bool,
}

#[derive(Clone)]
struct AggregateFieldPlan {
    offset: usize,
    size: usize,
    plan: Box<MarshalPlan>,
}

#[derive(Clone)]
struct RecordFieldPlan {
    label: Box<str>,
    offset: usize,
    size: usize,
    plan: Box<MarshalPlan>,
}

#[derive(Clone)]
struct OpaqueVariantPlan {
    name: Box<str>,
    tag: i64,
    field_count: usize,
    payload: Option<Box<MarshalPlan>>,
}

#[derive(Clone)]
enum MarshalPlanKind {
    Int,
    Float,
    Bool,
    Decimal,
    BigInt,
    Text,
    Bytes,
    InlineOption(ScalarOptionKind),
    NicheOption {
        payload: Box<MarshalPlan>,
    },
    Tuple {
        fields: Vec<AggregateFieldPlan>,
        size: usize,
        align: usize,
    },
    Record {
        fields: Vec<RecordFieldPlan>,
        size: usize,
        align: usize,
    },
    List {
        element: Box<MarshalPlan>,
        element_size: usize,
    },
    Set {
        element: Box<MarshalPlan>,
        element_size: usize,
    },
    Map {
        key: Box<MarshalPlan>,
        key_size: usize,
        value: Box<MarshalPlan>,
        value_size: usize,
    },
    Result {
        ok: Box<MarshalPlan>,
        err: Box<MarshalPlan>,
    },
    Validation {
        valid: Box<MarshalPlan>,
        invalid: Box<MarshalPlan>,
    },
    AnonymousDomain {
        carrier: Box<MarshalPlan>,
    },
    Opaque {
        item: aivi_hir::ItemId,
        type_name: Box<str>,
        variants: Vec<OpaqueVariantPlan>,
    },
    Domain,
}

#[derive(Clone)]
struct MarshalPlan {
    kind: MarshalPlanKind,
}

impl MarshalPlan {
    fn for_layout(program: &Program, layout: LayoutId) -> Option<Self> {
        let layout = program.layouts().get(layout)?;
        let kind = match (&layout.abi, &layout.kind) {
            (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Int)) => {
                MarshalPlanKind::Int
            }
            (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Float)) => {
                MarshalPlanKind::Float
            }
            (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Bool)) => {
                MarshalPlanKind::Bool
            }
            (AbiPassMode::ByReference, LayoutKind::Primitive(PrimitiveType::Decimal)) => {
                MarshalPlanKind::Decimal
            }
            (AbiPassMode::ByReference, LayoutKind::Primitive(PrimitiveType::BigInt)) => {
                MarshalPlanKind::BigInt
            }
            (AbiPassMode::ByReference, LayoutKind::Primitive(PrimitiveType::Text)) => {
                MarshalPlanKind::Text
            }
            (AbiPassMode::ByReference, LayoutKind::Primitive(PrimitiveType::Bytes)) => {
                MarshalPlanKind::Bytes
            }
            (AbiPassMode::ByValue, LayoutKind::Option { element }) => {
                let payload = program.layouts().get(*element)?;
                let kind = match (&payload.abi, &payload.kind) {
                    (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Int)) => {
                        ScalarOptionKind::Int
                    }
                    (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Float)) => {
                        ScalarOptionKind::Float
                    }
                    (AbiPassMode::ByValue, LayoutKind::Primitive(PrimitiveType::Bool)) => {
                        ScalarOptionKind::Bool
                    }
                    _ => return None,
                };
                MarshalPlanKind::InlineOption(kind)
            }
            (AbiPassMode::ByReference, LayoutKind::Option { element }) => {
                let payload_layout = program.layouts().get(*element)?;
                if payload_layout.abi != AbiPassMode::ByReference {
                    return None;
                }
                MarshalPlanKind::NicheOption {
                    payload: Box::new(Self::for_layout(program, *element)?),
                }
            }
            (AbiPassMode::ByReference, LayoutKind::Tuple(elements)) => {
                let (fields, size, align) = build_tuple_fields(program, elements)?;
                MarshalPlanKind::Tuple {
                    fields,
                    size,
                    align,
                }
            }
            (AbiPassMode::ByReference, LayoutKind::Record(fields)) => {
                let (fields, size, align) = build_record_fields(program, fields)?;
                MarshalPlanKind::Record {
                    fields,
                    size,
                    align,
                }
            }
            (AbiPassMode::ByReference, LayoutKind::List { element }) => MarshalPlanKind::List {
                element: Box::new(Self::for_layout(program, *element)?),
                element_size: cell_size_for_layout(program, *element)?,
            },
            (AbiPassMode::ByReference, LayoutKind::Set { element }) => MarshalPlanKind::Set {
                element: Box::new(Self::for_layout(program, *element)?),
                element_size: cell_size_for_layout(program, *element)?,
            },
            (AbiPassMode::ByReference, LayoutKind::Map { key, value }) => MarshalPlanKind::Map {
                key: Box::new(Self::for_layout(program, *key)?),
                key_size: cell_size_for_layout(program, *key)?,
                value: Box::new(Self::for_layout(program, *value)?),
                value_size: cell_size_for_layout(program, *value)?,
            },
            (AbiPassMode::ByReference, LayoutKind::Result { error, value }) => {
                MarshalPlanKind::Result {
                    ok: Box::new(Self::for_layout(program, *value)?),
                    err: Box::new(Self::for_layout(program, *error)?),
                }
            }
            (AbiPassMode::ByReference, LayoutKind::Validation { error, value }) => {
                MarshalPlanKind::Validation {
                    valid: Box::new(Self::for_layout(program, *value)?),
                    invalid: Box::new(Self::for_layout(program, *error)?),
                }
            }
            (AbiPassMode::ByReference, LayoutKind::AnonymousDomain { carrier, .. }) => {
                MarshalPlanKind::AnonymousDomain {
                    carrier: Box::new(Self::for_layout(program, *carrier)?),
                }
            }
            (
                AbiPassMode::ByReference,
                LayoutKind::Opaque {
                    item: Some(item),
                    name,
                    variants,
                    ..
                },
            ) if !variants.is_empty() => MarshalPlanKind::Opaque {
                item: *item,
                type_name: name.clone(),
                variants: variants
                    .iter()
                    .map(|variant| {
                        Some(OpaqueVariantPlan {
                            name: variant.name.clone(),
                            tag: crate::layout::opaque_variant_tag(variant.name.as_ref()),
                            field_count: variant.field_count,
                            payload: match variant.payload {
                                Some(payload) => {
                                    Some(Box::new(Self::for_layout(program, payload)?))
                                }
                                None => None,
                            },
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            },
            (AbiPassMode::ByReference, LayoutKind::Domain { .. }) => MarshalPlanKind::Domain,
            _ => return None,
        };
        Some(Self { kind })
    }

    fn matches(&self, value: &RuntimeValue) -> bool {
        match (&self.kind, value) {
            (MarshalPlanKind::Int, RuntimeValue::Int(_))
            | (MarshalPlanKind::Float, RuntimeValue::Float(_))
            | (MarshalPlanKind::Bool, RuntimeValue::Bool(_))
            | (MarshalPlanKind::Decimal, RuntimeValue::Decimal(_))
            | (MarshalPlanKind::BigInt, RuntimeValue::BigInt(_))
            | (MarshalPlanKind::Text, RuntimeValue::Text(_))
            | (MarshalPlanKind::Bytes, RuntimeValue::Bytes(_)) => true,
            (MarshalPlanKind::InlineOption(_), RuntimeValue::OptionNone) => true,
            (
                MarshalPlanKind::InlineOption(ScalarOptionKind::Int),
                RuntimeValue::OptionSome(value),
            ) => {
                matches!(value.as_ref(), RuntimeValue::Int(_))
            }
            (
                MarshalPlanKind::InlineOption(ScalarOptionKind::Float),
                RuntimeValue::OptionSome(value),
            ) => matches!(value.as_ref(), RuntimeValue::Float(_)),
            (
                MarshalPlanKind::InlineOption(ScalarOptionKind::Bool),
                RuntimeValue::OptionSome(value),
            ) => {
                matches!(value.as_ref(), RuntimeValue::Bool(_))
            }
            (MarshalPlanKind::NicheOption { .. }, RuntimeValue::OptionNone) => true,
            (MarshalPlanKind::NicheOption { payload }, RuntimeValue::OptionSome(value)) => {
                payload.matches(value.as_ref())
            }
            (MarshalPlanKind::Tuple { fields, .. }, RuntimeValue::Tuple(values)) => {
                fields.len() == values.len()
                    && fields
                        .iter()
                        .zip(values.iter())
                        .all(|(field, value)| field.plan.matches(value))
            }
            (MarshalPlanKind::Record { fields, .. }, RuntimeValue::Record(values)) => {
                fields.len() == values.len()
                    && fields.iter().zip(values.iter()).all(|(field, value)| {
                        field.label.as_ref() == value.label.as_ref()
                            && field.plan.matches(&value.value)
                    })
            }
            (MarshalPlanKind::List { element, .. }, RuntimeValue::List(values))
            | (MarshalPlanKind::Set { element, .. }, RuntimeValue::Set(values)) => {
                values.iter().all(|value| element.matches(value))
            }
            (MarshalPlanKind::Map { key, value, .. }, RuntimeValue::Map(entries)) => {
                entries.iter().all(|(entry_key, entry_value)| {
                    key.matches(entry_key) && value.matches(entry_value)
                })
            }
            (MarshalPlanKind::Result { ok, .. }, RuntimeValue::ResultOk(value)) => {
                ok.matches(value.as_ref())
            }
            (MarshalPlanKind::Result { err, .. }, RuntimeValue::ResultErr(value)) => {
                err.matches(value.as_ref())
            }
            (MarshalPlanKind::Validation { valid, .. }, RuntimeValue::ValidationValid(value)) => {
                valid.matches(value.as_ref())
            }
            (
                MarshalPlanKind::Validation { invalid, .. },
                RuntimeValue::ValidationInvalid(value),
            ) => invalid.matches(value.as_ref()),
            (MarshalPlanKind::AnonymousDomain { carrier }, value) => {
                carrier.matches(value) || matches!(value, RuntimeValue::SuffixedInteger { .. })
            }
            (
                MarshalPlanKind::Opaque {
                    item,
                    type_name,
                    variants,
                },
                RuntimeValue::Sum(value),
            ) => {
                value.item == *item
                    && value.type_name.as_ref() == type_name.as_ref()
                    && variants
                        .iter()
                        .find(|variant| variant.name.as_ref() == value.variant_name.as_ref())
                        .is_some_and(|variant| {
                            opaque_variant_matches_fields(
                                variant.field_count,
                                variant.payload.as_deref(),
                                &value.fields,
                            )
                        })
            }
            (MarshalPlanKind::Domain, value) => !matches!(value, RuntimeValue::Signal(_)),
            _ => false,
        }
    }

    fn pack_argument(
        &self,
        value: &RuntimeValue,
        arena: &mut AllocationArena,
        hints: &mut PackedValueHints,
    ) -> Option<AbiValue> {
        match &self.kind {
            MarshalPlanKind::Int => match value {
                RuntimeValue::Int(value) => Some(AbiValue::I64(*value)),
                _ => None,
            },
            MarshalPlanKind::Float => match value {
                RuntimeValue::Float(value) => Some(AbiValue::F64(value.to_f64())),
                _ => None,
            },
            MarshalPlanKind::Bool => match value {
                RuntimeValue::Bool(value) => Some(AbiValue::I8(i8::from(*value))),
                _ => None,
            },
            MarshalPlanKind::InlineOption(kind) => {
                Some(AbiValue::I128(pack_inline_option(*kind, value)?))
            }
            _ => Some(AbiValue::Pointer(self.pack_reference(value, arena, hints)?)),
        }
    }

    fn write_slot(
        &self,
        value: &RuntimeValue,
        cell: &mut [u8],
        arena: &mut AllocationArena,
        hints: &mut PackedValueHints,
    ) -> bool {
        cell.fill(0);
        let Some(bytes) = self.encode_cell_bytes(value, arena, hints) else {
            return false;
        };
        if cell.len() < bytes.len() {
            return false;
        }
        cell[..bytes.len()].copy_from_slice(&bytes);
        true
    }

    fn unpack_result(&self, value: AbiValue, hints: &PackedValueHints) -> Option<RuntimeValue> {
        match (&self.kind, value) {
            (MarshalPlanKind::Int, AbiValue::I64(value)) => Some(RuntimeValue::Int(value)),
            (MarshalPlanKind::Float, AbiValue::F64(value)) => {
                Some(RuntimeValue::Float(RuntimeFloat::new(value)?))
            }
            (MarshalPlanKind::Bool, AbiValue::I8(value)) => Some(RuntimeValue::Bool(value != 0)),
            (MarshalPlanKind::InlineOption(kind), AbiValue::I128(bits)) => {
                unpack_inline_option(*kind, bits)
            }
            (_, AbiValue::Pointer(value)) => self.unpack_reference(value, hints),
            _ => None,
        }
    }

    fn cell_size(&self) -> usize {
        match self.kind {
            MarshalPlanKind::Int | MarshalPlanKind::Float => 8,
            MarshalPlanKind::Bool => 1,
            MarshalPlanKind::InlineOption(_) => 16,
            _ => std::mem::size_of::<usize>(),
        }
    }

    fn cell_align(&self) -> usize {
        match self.kind {
            MarshalPlanKind::Bool => 1,
            MarshalPlanKind::InlineOption(_) => 16,
            _ => self.cell_size().max(1),
        }
    }

    fn encode_cell_bytes(
        &self,
        value: &RuntimeValue,
        arena: &mut AllocationArena,
        hints: &mut PackedValueHints,
    ) -> Option<Vec<u8>> {
        match &self.kind {
            MarshalPlanKind::Int => match value {
                RuntimeValue::Int(value) => Some(value.to_ne_bytes().to_vec()),
                _ => None,
            },
            MarshalPlanKind::Float => match value {
                RuntimeValue::Float(value) => Some(value.to_f64().to_bits().to_ne_bytes().to_vec()),
                _ => None,
            },
            MarshalPlanKind::Bool => match value {
                RuntimeValue::Bool(value) => Some(vec![u8::from(*value)]),
                _ => None,
            },
            MarshalPlanKind::InlineOption(kind) => {
                Some(pack_inline_option(*kind, value)?.to_ne_bytes().to_vec())
            }
            _ => Some(pointer_bytes(self.pack_reference(value, arena, hints)?)),
        }
    }

    fn decode_cell_bytes(&self, bytes: &[u8], hints: &PackedValueHints) -> Option<RuntimeValue> {
        match &self.kind {
            MarshalPlanKind::Int => Some(RuntimeValue::Int(i64::from_ne_bytes(
                bytes.try_into().ok()?,
            ))),
            MarshalPlanKind::Float => Some(RuntimeValue::Float(RuntimeFloat::new(
                f64::from_bits(u64::from_ne_bytes(bytes.try_into().ok()?)),
            )?)),
            MarshalPlanKind::Bool => Some(RuntimeValue::Bool(bytes.first().copied()? != 0)),
            MarshalPlanKind::InlineOption(kind) => {
                unpack_inline_option(*kind, u128::from_ne_bytes(bytes.try_into().ok()?))
            }
            _ => self.unpack_reference(pointer_from_bytes(bytes)?, hints),
        }
    }

    fn pack_reference(
        &self,
        value: &RuntimeValue,
        arena: &mut AllocationArena,
        hints: &mut PackedValueHints,
    ) -> Option<*const c_void> {
        let pointer = match &self.kind {
            MarshalPlanKind::Decimal => match value {
                RuntimeValue::Decimal(value) => {
                    arena.store_raw_bytes_aligned(value.encode_constant_bytes().as_ref(), 16)
                }
                _ => return None,
            },
            MarshalPlanKind::BigInt => match value {
                RuntimeValue::BigInt(value) => {
                    arena.store_raw_bytes_aligned(value.encode_constant_bytes().as_ref(), 8)
                }
                _ => return None,
            },
            MarshalPlanKind::Text => match value {
                RuntimeValue::Text(value) => encode_len_prefixed_bytes(value.as_bytes(), arena),
                _ => return None,
            },
            MarshalPlanKind::Bytes => match value {
                RuntimeValue::Bytes(value) => encode_len_prefixed_bytes(value.as_ref(), arena),
                _ => return None,
            },
            MarshalPlanKind::NicheOption { payload } => match value {
                RuntimeValue::OptionNone => ptr::null(),
                RuntimeValue::OptionSome(value) => {
                    payload.pack_reference(value.as_ref(), arena, hints)?
                }
                _ => return None,
            },
            MarshalPlanKind::Tuple {
                fields,
                size,
                align,
            } => {
                let RuntimeValue::Tuple(values) = value else {
                    return None;
                };
                if values.len() != fields.len() {
                    return None;
                }
                let mut encoded = vec![0u8; *size];
                for (field, value) in fields.iter().zip(values.iter()) {
                    let bytes = field.plan.encode_cell_bytes(value, arena, hints)?;
                    encoded[field.offset..field.offset + field.size].copy_from_slice(&bytes);
                }
                arena.store_raw_bytes_aligned(&encoded, *align)
            }
            MarshalPlanKind::Record {
                fields,
                size,
                align,
            } => {
                let RuntimeValue::Record(values) = value else {
                    return None;
                };
                if values.len() != fields.len() {
                    return None;
                }
                let mut encoded = vec![0u8; *size];
                for (field, value) in fields.iter().zip(values.iter()) {
                    if field.label.as_ref() != value.label.as_ref() {
                        return None;
                    }
                    let bytes = field.plan.encode_cell_bytes(&value.value, arena, hints)?;
                    encoded[field.offset..field.offset + field.size].copy_from_slice(&bytes);
                }
                arena.store_raw_bytes_aligned(&encoded, *align)
            }
            MarshalPlanKind::List {
                element,
                element_size,
            } => {
                let RuntimeValue::List(values) = value else {
                    return None;
                };
                let mut encoded =
                    Vec::with_capacity(values.len().checked_mul(*element_size).unwrap_or_default());
                for value in values {
                    encoded.extend_from_slice(&element.encode_cell_bytes(value, arena, hints)?);
                }
                encode_marshaled_sequence(values.len(), *element_size, &encoded, arena)?
            }
            MarshalPlanKind::Set {
                element,
                element_size,
            } => {
                let RuntimeValue::Set(values) = value else {
                    return None;
                };
                let mut encoded =
                    Vec::with_capacity(values.len().checked_mul(*element_size).unwrap_or_default());
                for value in values {
                    encoded.extend_from_slice(&element.encode_cell_bytes(value, arena, hints)?);
                }
                encode_marshaled_sequence(values.len(), *element_size, &encoded, arena)?
            }
            MarshalPlanKind::Map {
                key,
                key_size,
                value: value_plan,
                value_size,
            } => {
                let RuntimeValue::Map(entries) = value else {
                    return None;
                };
                let entry_size = key_size.checked_add(*value_size)?;
                let mut encoded =
                    Vec::with_capacity(entries.len().checked_mul(entry_size).unwrap_or_default());
                for (entry_key, entry_value) in entries.iter() {
                    encoded.extend_from_slice(&key.encode_cell_bytes(entry_key, arena, hints)?);
                    encoded.extend_from_slice(&value_plan.encode_cell_bytes(
                        entry_value,
                        arena,
                        hints,
                    )?);
                }
                encode_marshaled_map(entries.len(), *key_size, *value_size, &encoded, arena)?
            }
            MarshalPlanKind::Result { ok, err } => {
                pack_tagged_payload(value, ok, err, RuntimeValueTag::Result, arena, hints)?
            }
            MarshalPlanKind::Validation { valid, invalid } => pack_tagged_payload(
                value,
                valid,
                invalid,
                RuntimeValueTag::Validation,
                arena,
                hints,
            )?,
            MarshalPlanKind::AnonymousDomain { carrier } => match value {
                RuntimeValue::SuffixedInteger { raw, .. } => {
                    let parsed = raw.parse::<i64>().ok()?;
                    pack_int_reference(parsed, arena)
                }
                value if carrier.matches(value) => carrier.pack_reference(value, arena, hints)?,
                _ => return None,
            },
            MarshalPlanKind::Opaque {
                item,
                type_name,
                variants,
            } => {
                let RuntimeValue::Sum(sum) = value else {
                    return None;
                };
                if sum.item != *item || sum.type_name.as_ref() != type_name.as_ref() {
                    return None;
                }
                let variant = variants
                    .iter()
                    .find(|variant| variant.name.as_ref() == sum.variant_name.as_ref())?;
                let payload = encode_opaque_variant_payload(
                    variant.field_count,
                    variant.payload.as_deref(),
                    &sum.fields,
                    arena,
                    hints,
                )?;
                let mut encoded = vec![0u8; 8 + payload.len()];
                encoded[..8].copy_from_slice(&variant.tag.to_ne_bytes());
                encoded[8..8 + payload.len()].copy_from_slice(&payload);
                arena.store_raw_bytes_aligned(&encoded, 8)
            }
            MarshalPlanKind::Domain => pack_erased_domain_value(value, arena)?,
            MarshalPlanKind::Int
            | MarshalPlanKind::Float
            | MarshalPlanKind::Bool
            | MarshalPlanKind::InlineOption(_) => return None,
        };
        hints.remember(pointer, value);
        Some(pointer)
    }

    fn unpack_reference(
        &self,
        pointer: *const c_void,
        hints: &PackedValueHints,
    ) -> Option<RuntimeValue> {
        if pointer.is_null() {
            return match self.kind {
                MarshalPlanKind::NicheOption { .. } => Some(RuntimeValue::OptionNone),
                _ => None,
            };
        }
        if let Some(value) = hints.lookup_matching(self, pointer) {
            return Some(value);
        }
        match &self.kind {
            MarshalPlanKind::Decimal => {
                Some(RuntimeValue::Decimal(RuntimeDecimal::from_constant_bytes(
                    read_decimal_constant_bytes(pointer)?.as_ref(),
                )?))
            }
            MarshalPlanKind::BigInt => Some(RuntimeValue::BigInt(
                RuntimeBigInt::from_constant_bytes(read_bigint_constant_bytes(pointer)?.as_ref())?,
            )),
            MarshalPlanKind::Text => decode_text(pointer),
            MarshalPlanKind::Bytes => {
                Some(RuntimeValue::Bytes(decode_len_prefixed_bytes(pointer)?))
            }
            MarshalPlanKind::NicheOption { payload } => {
                if pointer.is_null() {
                    Some(RuntimeValue::OptionNone)
                } else {
                    Some(RuntimeValue::OptionSome(Box::new(
                        payload.unpack_reference(pointer, hints)?,
                    )))
                }
            }
            MarshalPlanKind::Tuple { fields, .. } => {
                let mut values = Vec::with_capacity(fields.len());
                for field in fields {
                    let bytes = read_marshaled_field(pointer, field.offset, field.size)?;
                    values.push(field.plan.decode_cell_bytes(bytes.as_ref(), hints)?);
                }
                Some(RuntimeValue::Tuple(values))
            }
            MarshalPlanKind::Record { fields, .. } => {
                let mut values = Vec::with_capacity(fields.len());
                for field in fields {
                    let bytes = read_marshaled_field(pointer, field.offset, field.size)?;
                    values.push(RuntimeRecordField {
                        label: field.label.clone(),
                        value: field.plan.decode_cell_bytes(bytes.as_ref(), hints)?,
                    });
                }
                Some(RuntimeValue::Record(values))
            }
            MarshalPlanKind::List {
                element,
                element_size,
            } => {
                let decoded = decode_marshaled_sequence(pointer)?;
                if decoded.element_size != *element_size {
                    return None;
                }
                let mut values = Vec::with_capacity(decoded.count);
                for chunk in decoded.bytes.chunks_exact(*element_size) {
                    values.push(element.decode_cell_bytes(chunk, hints)?);
                }
                Some(RuntimeValue::List(values))
            }
            MarshalPlanKind::Set {
                element,
                element_size,
            } => {
                let decoded = decode_marshaled_sequence(pointer)?;
                if decoded.element_size != *element_size {
                    return None;
                }
                let mut values = Vec::with_capacity(decoded.count);
                for chunk in decoded.bytes.chunks_exact(*element_size) {
                    values.push(element.decode_cell_bytes(chunk, hints)?);
                }
                Some(RuntimeValue::Set(values))
            }
            MarshalPlanKind::Map {
                key,
                key_size,
                value: value_plan,
                value_size,
            } => {
                let decoded = decode_marshaled_map(pointer)?;
                if decoded.key_size != *key_size || decoded.value_size != *value_size {
                    return None;
                }
                let entry_size = key_size.checked_add(*value_size)?;
                let mut entries = Vec::with_capacity(decoded.count);
                for chunk in decoded.bytes.chunks_exact(entry_size) {
                    let key_bytes = &chunk[..*key_size];
                    let value_bytes = &chunk[*key_size..entry_size];
                    entries.push(RuntimeMapEntry {
                        key: key.decode_cell_bytes(key_bytes, hints)?,
                        value: value_plan.decode_cell_bytes(value_bytes, hints)?,
                    });
                }
                Some(RuntimeValue::Map(RuntimeMap::from_entries(entries)))
            }
            MarshalPlanKind::Result { ok, err } => {
                unpack_tagged_payload(pointer, ok, err, RuntimeValueTag::Result, hints)
            }
            MarshalPlanKind::Validation { valid, invalid } => {
                unpack_tagged_payload(pointer, valid, invalid, RuntimeValueTag::Validation, hints)
            }
            MarshalPlanKind::AnonymousDomain { carrier } => {
                carrier.unpack_reference(pointer, hints)
            }
            MarshalPlanKind::Opaque {
                item,
                type_name,
                variants,
            } => {
                let tag_bytes = read_marshaled_field(pointer, 0, 8)?;
                let tag = i64::from_ne_bytes(tag_bytes.as_ref().try_into().ok()?);
                let variant = variants.iter().find(|variant| variant.tag == tag)?;
                let fields = decode_opaque_variant_fields(
                    variant.field_count,
                    variant.payload.as_deref(),
                    pointer,
                    hints,
                )?;
                Some(RuntimeValue::Sum(crate::RuntimeSumValue {
                    item: *item,
                    type_name: type_name.clone(),
                    variant_name: variant.name.clone(),
                    fields,
                }))
            }
            MarshalPlanKind::Domain => None,
            MarshalPlanKind::Int
            | MarshalPlanKind::Float
            | MarshalPlanKind::Bool
            | MarshalPlanKind::InlineOption(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
enum RuntimeValueTag {
    Result,
    Validation,
}

fn build_tuple_fields(
    program: &Program,
    elements: &[LayoutId],
) -> Option<(Vec<AggregateFieldPlan>, usize, usize)> {
    let mut fields = Vec::with_capacity(elements.len());
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for layout in elements {
        let plan = Box::new(MarshalPlan::for_layout(program, *layout)?);
        let align = plan.cell_align();
        let size = plan.cell_size();
        max_align = max_align.max(align);
        offset = align_offset(offset, align);
        fields.push(AggregateFieldPlan { offset, size, plan });
        offset = offset.checked_add(size)?;
    }
    Some((fields, offset, max_align))
}

fn build_record_fields(
    program: &Program,
    fields: &[crate::RecordFieldLayout],
) -> Option<(Vec<RecordFieldPlan>, usize, usize)> {
    let mut plans = Vec::with_capacity(fields.len());
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for field in fields {
        let plan = Box::new(MarshalPlan::for_layout(program, field.layout)?);
        let align = plan.cell_align();
        let size = plan.cell_size();
        max_align = max_align.max(align);
        offset = align_offset(offset, align);
        plans.push(RecordFieldPlan {
            label: field.name.clone(),
            offset,
            size,
            plan,
        });
        offset = offset.checked_add(size)?;
    }
    Some((plans, offset, max_align))
}

fn cell_size_for_layout(program: &Program, layout: LayoutId) -> Option<usize> {
    Some(MarshalPlan::for_layout(program, layout)?.cell_size())
}

fn align_offset(offset: usize, align: usize) -> usize {
    let align = align.max(1);
    (offset + (align - 1)) & !(align - 1)
}

fn pointer_bytes(pointer: *const c_void) -> Vec<u8> {
    (pointer as usize).to_ne_bytes().to_vec()
}

fn pointer_from_bytes(bytes: &[u8]) -> Option<*const c_void> {
    let raw: [u8; std::mem::size_of::<usize>()] = bytes.try_into().ok()?;
    Some(usize::from_ne_bytes(raw) as *const c_void)
}

fn pack_int_reference(value: i64, arena: &mut AllocationArena) -> *const c_void {
    arena.store_raw_bytes_aligned(&value.to_ne_bytes(), 8)
}

fn pack_float_reference(value: RuntimeFloat, arena: &mut AllocationArena) -> *const c_void {
    arena.store_raw_bytes_aligned(&value.to_f64().to_bits().to_ne_bytes(), 8)
}

fn pack_bool_reference(value: bool, arena: &mut AllocationArena) -> *const c_void {
    arena.store_raw_bytes_aligned(&[u8::from(value)], 1)
}

fn pack_inline_option(kind: ScalarOptionKind, value: &RuntimeValue) -> Option<u128> {
    match (kind, value) {
        (_, RuntimeValue::OptionNone) => Some(0),
        (ScalarOptionKind::Int, RuntimeValue::OptionSome(value)) => match value.as_ref() {
            RuntimeValue::Int(value) => Some(encode_inline_option_bits(*value as u64)),
            _ => None,
        },
        (ScalarOptionKind::Float, RuntimeValue::OptionSome(value)) => match value.as_ref() {
            RuntimeValue::Float(value) => Some(encode_inline_option_bits(value.to_f64().to_bits())),
            _ => None,
        },
        (ScalarOptionKind::Bool, RuntimeValue::OptionSome(value)) => match value.as_ref() {
            RuntimeValue::Bool(value) => Some(encode_inline_option_bits(u64::from(*value))),
            _ => None,
        },
        _ => None,
    }
}

fn unpack_inline_option(kind: ScalarOptionKind, bits: u128) -> Option<RuntimeValue> {
    match kind {
        ScalarOptionKind::Int => decode_inline_int_option(bits),
        ScalarOptionKind::Float => decode_inline_float_option(bits),
        ScalarOptionKind::Bool => decode_inline_bool_option(bits),
    }
}

fn pack_tagged_payload(
    value: &RuntimeValue,
    primary: &MarshalPlan,
    alternate: &MarshalPlan,
    tag_kind: RuntimeValueTag,
    arena: &mut AllocationArena,
    hints: &mut PackedValueHints,
) -> Option<*const c_void> {
    let (tag, payload_plan, payload_value) = match (tag_kind, value) {
        (RuntimeValueTag::Result, RuntimeValue::ResultOk(value)) => (0i64, primary, value.as_ref()),
        (RuntimeValueTag::Result, RuntimeValue::ResultErr(value)) => {
            (1i64, alternate, value.as_ref())
        }
        (RuntimeValueTag::Validation, RuntimeValue::ValidationValid(value)) => {
            (0i64, primary, value.as_ref())
        }
        (RuntimeValueTag::Validation, RuntimeValue::ValidationInvalid(value)) => {
            (1i64, alternate, value.as_ref())
        }
        _ => return None,
    };
    let payload = payload_plan.encode_cell_bytes(payload_value, arena, hints)?;
    let mut encoded = vec![0u8; 8 + payload.len()];
    encoded[..8].copy_from_slice(&tag.to_ne_bytes());
    encoded[8..8 + payload.len()].copy_from_slice(&payload);
    Some(arena.store_raw_bytes_aligned(&encoded, 8))
}

fn unpack_tagged_payload(
    pointer: *const c_void,
    primary: &MarshalPlan,
    alternate: &MarshalPlan,
    tag_kind: RuntimeValueTag,
    hints: &PackedValueHints,
) -> Option<RuntimeValue> {
    let tag_bytes = read_marshaled_field(pointer, 0, 8)?;
    let tag = i64::from_ne_bytes(tag_bytes.as_ref().try_into().ok()?);
    match (tag_kind, tag) {
        (RuntimeValueTag::Result, 0) => {
            let payload = read_marshaled_field(pointer, 8, primary.cell_size())?;
            Some(RuntimeValue::ResultOk(Box::new(
                primary.decode_cell_bytes(payload.as_ref(), hints)?,
            )))
        }
        (RuntimeValueTag::Result, 1) => {
            let payload = read_marshaled_field(pointer, 8, alternate.cell_size())?;
            Some(RuntimeValue::ResultErr(Box::new(
                alternate.decode_cell_bytes(payload.as_ref(), hints)?,
            )))
        }
        (RuntimeValueTag::Validation, 0) => {
            let payload = read_marshaled_field(pointer, 8, primary.cell_size())?;
            Some(RuntimeValue::ValidationValid(Box::new(
                primary.decode_cell_bytes(payload.as_ref(), hints)?,
            )))
        }
        (RuntimeValueTag::Validation, 1) => {
            let payload = read_marshaled_field(pointer, 8, alternate.cell_size())?;
            Some(RuntimeValue::ValidationInvalid(Box::new(
                alternate.decode_cell_bytes(payload.as_ref(), hints)?,
            )))
        }
        _ => None,
    }
}

fn opaque_variant_matches_fields(
    field_count: usize,
    payload: Option<&MarshalPlan>,
    fields: &[RuntimeValue],
) -> bool {
    match (field_count, payload, fields) {
        (0, None, []) => true,
        (1, Some(payload), [field]) => payload.matches(field),
        (count, Some(payload), fields) if count > 1 && count == fields.len() => {
            payload.matches(&RuntimeValue::Tuple(fields.to_vec()))
        }
        _ => false,
    }
}

fn encode_opaque_variant_payload(
    field_count: usize,
    payload: Option<&MarshalPlan>,
    fields: &[RuntimeValue],
    arena: &mut AllocationArena,
    hints: &mut PackedValueHints,
) -> Option<Vec<u8>> {
    match (field_count, payload, fields) {
        (0, None, []) => Some(Vec::new()),
        (1, Some(payload), [field]) => payload.encode_cell_bytes(field, arena, hints),
        (count, Some(payload), fields) if count > 1 && count == fields.len() => {
            payload.encode_cell_bytes(&RuntimeValue::Tuple(fields.to_vec()), arena, hints)
        }
        _ => None,
    }
}

fn decode_opaque_variant_fields(
    field_count: usize,
    payload: Option<&MarshalPlan>,
    pointer: *const c_void,
    hints: &PackedValueHints,
) -> Option<Vec<RuntimeValue>> {
    match (field_count, payload) {
        (0, None) => Some(Vec::new()),
        (1, Some(payload)) => {
            let payload_bytes = read_marshaled_field(pointer, 8, payload.cell_size())?;
            Some(vec![
                payload.decode_cell_bytes(payload_bytes.as_ref(), hints)?,
            ])
        }
        (count, Some(payload)) if count > 1 => {
            let payload_bytes = read_marshaled_field(pointer, 8, payload.cell_size())?;
            let RuntimeValue::Tuple(fields) =
                payload.decode_cell_bytes(payload_bytes.as_ref(), hints)?
            else {
                return None;
            };
            if fields.len() != count {
                return None;
            }
            Some(fields)
        }
        _ => None,
    }
}

fn pack_erased_domain_value(
    value: &RuntimeValue,
    arena: &mut AllocationArena,
) -> Option<*const c_void> {
    match value {
        RuntimeValue::Int(value) => Some(pack_int_reference(*value, arena)),
        RuntimeValue::Float(value) => Some(pack_float_reference(*value, arena)),
        RuntimeValue::Bool(value) => Some(pack_bool_reference(*value, arena)),
        RuntimeValue::Text(value) => Some(encode_len_prefixed_bytes(value.as_bytes(), arena)),
        RuntimeValue::Bytes(value) => Some(encode_len_prefixed_bytes(value.as_ref(), arena)),
        RuntimeValue::Decimal(value) => {
            Some(arena.store_raw_bytes_aligned(value.encode_constant_bytes().as_ref(), 16))
        }
        RuntimeValue::BigInt(value) => {
            Some(arena.store_raw_bytes_aligned(value.encode_constant_bytes().as_ref(), 8))
        }
        RuntimeValue::OptionNone => Some(ptr::null()),
        RuntimeValue::OptionSome(value) => pack_erased_domain_value(value.as_ref(), arena),
        RuntimeValue::SuffixedInteger { raw, .. } => {
            Some(pack_int_reference(raw.parse::<i64>().ok()?, arena))
        }
        _ => None,
    }
}

#[derive(Clone)]
struct LastKernelCall {
    kernel_id: KernelId,
    input_subject: Option<RuntimeValue>,
    environment: Box<[RuntimeValue]>,
    result: RuntimeValue,
    result_layout: LayoutId,
}

enum CompiledKernelFailure {
    Evaluation(EvaluationError),
    Fallback,
}

fn decode_text(pointer: *const c_void) -> Option<RuntimeValue> {
    let bytes = decode_len_prefixed_bytes(pointer)?;
    let text = String::from_utf8(bytes.into_vec()).ok()?;
    Some(RuntimeValue::Text(text.into_boxed_str()))
}

fn strip_signal_wrappers(mut value: &RuntimeValue) -> &RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = inner.as_ref();
    }
    value
}

const fn encode_inline_option_bits(payload: u64) -> u128 {
    ((payload as u128) << 64) | 1
}

fn decode_inline_int_option(bits: u128) -> Option<RuntimeValue> {
    if (bits as u64) == 0 {
        return Some(RuntimeValue::OptionNone);
    }
    Some(RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(
        (bits >> 64) as u64 as i64,
    ))))
}

fn decode_inline_float_option(bits: u128) -> Option<RuntimeValue> {
    if (bits as u64) == 0 {
        return Some(RuntimeValue::OptionNone);
    }
    Some(RuntimeValue::OptionSome(Box::new(RuntimeValue::Float(
        RuntimeFloat::new(f64::from_bits((bits >> 64) as u64))?,
    ))))
}

fn decode_inline_bool_option(bits: u128) -> Option<RuntimeValue> {
    if (bits as u64) == 0 {
        return Some(RuntimeValue::OptionNone);
    }
    Some(RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(
        ((bits >> 64) as u64) != 0,
    ))))
}

fn record_call(profile: &mut EvaluationCallProfile, elapsed: Duration, cache_hit: bool) {
    profile.calls += 1;
    if cache_hit {
        profile.cache_hits += 1;
    }
    profile.total_time += elapsed;
    profile.max_time = profile.max_time.max(elapsed);
}
