use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};

use aivi_ffi_call::{
    AbiValue, AllocationArena, decode_len_prefixed_bytes, encode_len_prefixed_bytes,
    with_active_arena,
};

use crate::{
    BackendExecutionEngine, BackendExecutionEngineKind, EvalFrame, EvaluationCallProfile,
    EvaluationError, ItemId, KernelEvaluationProfile, KernelEvaluator, KernelFingerprint, KernelId,
    LayoutId, LayoutKind, PrimitiveType, Program, RuntimeCallable, RuntimeFloat, RuntimeValue,
    TASK_COMPOSITION_KERNEL_ID, TaskFunctionApplier,
    codegen::{CompiledJitKernel, compile_kernel_jit},
    compute_kernel_fingerprint,
    program::ItemKind,
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
    pub(crate) fn new(program: &'a Program) -> Self {
        Self::with_profile(program, false)
    }

    pub(crate) fn new_profiled(program: &'a Program) -> Self {
        Self::with_profile(program, true)
    }

    fn with_profile(program: &'a Program, profiled: bool) -> Self {
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
                    validate_compiled_inputs(
                        kernel_id,
                        &kernel,
                        compiled,
                        input_subject,
                        environment,
                    )?;
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

    fn apply_runtime_callable(
        &mut self,
        kernel_id: KernelId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let result = self
            .fallback
            .apply_runtime_callable(kernel_id, callee, arguments, globals);
        self.refresh_profile();
        result
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

fn validate_compiled_inputs(
    kernel_id: KernelId,
    kernel: &crate::Kernel,
    plan: &CompiledKernelPlan,
    input_subject: Option<&RuntimeValue>,
    environment: &[RuntimeValue],
) -> Result<(), EvaluationError> {
    match (&plan.input_plan, input_subject) {
        (Some(expected), Some(value)) if expected.matches(value) => {}
        (Some(_), Some(value)) => {
            return Err(EvaluationError::KernelInputLayoutMismatch {
                kernel: kernel_id,
                expected: kernel
                    .input_subject
                    .expect("compiled subject plan implies subject"),
                found: value.clone(),
            });
        }
        (Some(_), None) => {
            return Err(EvaluationError::MissingInputSubject { kernel: kernel_id });
        }
        (None, Some(_)) => {
            return Err(EvaluationError::UnexpectedInputSubject { kernel: kernel_id });
        }
        (None, None) => {}
    }
    if environment.len() != plan.environment_plans.len() {
        return Err(EvaluationError::KernelEnvironmentCountMismatch {
            kernel: kernel_id,
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
                kernel: kernel_id,
                slot: crate::EnvSlotId::from_raw(index as u32),
                expected: kernel.environment[index],
                found: value.clone(),
            });
        }
    }
    Ok(())
}

fn execute_compiled_kernel(
    kernel_id: KernelId,
    plan: &mut CompiledKernelPlan,
    input_subject: Option<&RuntimeValue>,
    environment: &[RuntimeValue],
    globals: &BTreeMap<ItemId, RuntimeValue>,
) -> Result<RuntimeValue, CompiledKernelFailure> {
    let arena = Rc::new(RefCell::new(AllocationArena::new()));
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
            if !slot_plan.write_slot(strip_signal_wrappers(value), &mut slot.cell, &mut arena_mut) {
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
            if !slot_plan.write_slot(strip_signal_wrappers(value), &mut slot.cell, &mut arena_mut) {
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
            let Some(arg) = input_plan.pack_argument(value, &mut arena_mut) else {
                return Err(CompiledKernelFailure::Fallback);
            };
            args.push(arg);
        }
        for (slot_plan, value) in plan.environment_plans.iter().zip(environment.iter()) {
            let Some(arg) = slot_plan.pack_argument(value, &mut arena_mut) else {
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
        .unpack_result(call_result)
        .ok_or(CompiledKernelFailure::Fallback)
}

enum CachedKernelPlan {
    Compiled(CompiledKernelPlan),
    Fallback,
}

impl CachedKernelPlan {
    fn build(program: &Program, kernel_id: KernelId) -> Self {
        let Some(compiled) = CompiledKernelPlan::compile(program, kernel_id) else {
            return Self::Fallback;
        };
        Self::Compiled(compiled)
    }
}

struct CompiledKernelPlan {
    artifact: CompiledJitKernel,
    input_plan: Option<MarshalPlan>,
    environment_plans: Vec<MarshalPlan>,
    result_plan: MarshalPlan,
    signal_slot_plans: Vec<MarshalPlan>,
    imported_item_slot_plans: Vec<MarshalPlan>,
}

impl CompiledKernelPlan {
    fn compile(program: &Program, kernel_id: KernelId) -> Option<Self> {
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
        let artifact = compile_kernel_jit(program, kernel_id).ok()?;
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
            artifact,
            input_plan,
            environment_plans,
            result_plan,
            signal_slot_plans,
            imported_item_slot_plans,
        })
    }
}

#[derive(Clone, Copy)]
enum MarshalPlan {
    Int,
    Float,
    Bool,
    Text,
    Bytes,
    OptionText,
    OptionBytes,
}

impl MarshalPlan {
    fn for_layout(program: &Program, layout: LayoutId) -> Option<Self> {
        match &program.layouts().get(layout)?.kind {
            LayoutKind::Primitive(PrimitiveType::Int) => Some(Self::Int),
            LayoutKind::Primitive(PrimitiveType::Float) => Some(Self::Float),
            LayoutKind::Primitive(PrimitiveType::Bool) => Some(Self::Bool),
            LayoutKind::Primitive(PrimitiveType::Text) => Some(Self::Text),
            LayoutKind::Primitive(PrimitiveType::Bytes) => Some(Self::Bytes),
            LayoutKind::Option { element } => match &program.layouts().get(*element)?.kind {
                LayoutKind::Primitive(PrimitiveType::Text) => Some(Self::OptionText),
                LayoutKind::Primitive(PrimitiveType::Bytes) => Some(Self::OptionBytes),
                _ => None,
            },
            _ => None,
        }
    }

    fn matches(self, value: &RuntimeValue) -> bool {
        match (self, value) {
            (Self::Int, RuntimeValue::Int(_))
            | (Self::Float, RuntimeValue::Float(_))
            | (Self::Bool, RuntimeValue::Bool(_))
            | (Self::Text, RuntimeValue::Text(_))
            | (Self::Bytes, RuntimeValue::Bytes(_))
            | (Self::OptionText, RuntimeValue::OptionNone)
            | (Self::OptionBytes, RuntimeValue::OptionNone) => true,
            (Self::OptionText, RuntimeValue::OptionSome(value)) => {
                matches!(value.as_ref(), RuntimeValue::Text(_))
            }
            (Self::OptionBytes, RuntimeValue::OptionSome(value)) => {
                matches!(value.as_ref(), RuntimeValue::Bytes(_))
            }
            _ => false,
        }
    }

    fn pack_argument(self, value: &RuntimeValue, arena: &mut AllocationArena) -> Option<AbiValue> {
        match (self, value) {
            (Self::Int, RuntimeValue::Int(value)) => Some(AbiValue::I64(*value)),
            (Self::Float, RuntimeValue::Float(value)) => Some(AbiValue::F64(value.to_f64())),
            (Self::Bool, RuntimeValue::Bool(value)) => Some(AbiValue::I8(i8::from(*value))),
            (Self::Text, RuntimeValue::Text(value)) => Some(AbiValue::Pointer(
                encode_len_prefixed_bytes(value.as_bytes(), arena),
            )),
            (Self::Bytes, RuntimeValue::Bytes(value)) => Some(AbiValue::Pointer(
                encode_len_prefixed_bytes(value.as_ref(), arena),
            )),
            (Self::OptionText, RuntimeValue::OptionNone)
            | (Self::OptionBytes, RuntimeValue::OptionNone) => Some(AbiValue::Pointer(ptr::null())),
            (Self::OptionText, RuntimeValue::OptionSome(value)) => match value.as_ref() {
                RuntimeValue::Text(value) => Some(AbiValue::Pointer(encode_len_prefixed_bytes(
                    value.as_bytes(),
                    arena,
                ))),
                _ => None,
            },
            (Self::OptionBytes, RuntimeValue::OptionSome(value)) => match value.as_ref() {
                RuntimeValue::Bytes(value) => Some(AbiValue::Pointer(encode_len_prefixed_bytes(
                    value.as_ref(),
                    arena,
                ))),
                _ => None,
            },
            _ => None,
        }
    }

    fn write_slot(
        self,
        value: &RuntimeValue,
        cell: &mut [u8],
        arena: &mut AllocationArena,
    ) -> bool {
        cell.fill(0);
        match (self, value) {
            (Self::Int, RuntimeValue::Int(value)) => {
                cell[..8].copy_from_slice(&value.to_ne_bytes());
                true
            }
            (Self::Float, RuntimeValue::Float(value)) => {
                cell[..8].copy_from_slice(&value.to_f64().to_bits().to_ne_bytes());
                true
            }
            (Self::Bool, RuntimeValue::Bool(value)) => {
                cell[0] = u8::from(*value);
                true
            }
            (Self::Text, RuntimeValue::Text(value)) => {
                write_pointer_cell(cell, encode_len_prefixed_bytes(value.as_bytes(), arena))
            }
            (Self::Bytes, RuntimeValue::Bytes(value)) => {
                write_pointer_cell(cell, encode_len_prefixed_bytes(value.as_ref(), arena))
            }
            (Self::OptionText, RuntimeValue::OptionNone)
            | (Self::OptionBytes, RuntimeValue::OptionNone) => {
                write_pointer_cell(cell, ptr::null())
            }
            (Self::OptionText, RuntimeValue::OptionSome(value)) => match value.as_ref() {
                RuntimeValue::Text(value) => {
                    write_pointer_cell(cell, encode_len_prefixed_bytes(value.as_bytes(), arena))
                }
                _ => false,
            },
            (Self::OptionBytes, RuntimeValue::OptionSome(value)) => match value.as_ref() {
                RuntimeValue::Bytes(value) => {
                    write_pointer_cell(cell, encode_len_prefixed_bytes(value.as_ref(), arena))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn unpack_result(self, value: AbiValue) -> Option<RuntimeValue> {
        match (self, value) {
            (Self::Int, AbiValue::I64(value)) => Some(RuntimeValue::Int(value)),
            (Self::Float, AbiValue::F64(value)) => {
                Some(RuntimeValue::Float(RuntimeFloat::new(value)?))
            }
            (Self::Bool, AbiValue::I8(value)) => Some(RuntimeValue::Bool(value != 0)),
            (Self::Text, AbiValue::Pointer(value)) => decode_text(value),
            (Self::Bytes, AbiValue::Pointer(value)) => {
                Some(RuntimeValue::Bytes(decode_len_prefixed_bytes(value)?))
            }
            (Self::OptionText, AbiValue::Pointer(value)) => {
                if value.is_null() {
                    Some(RuntimeValue::OptionNone)
                } else {
                    Some(RuntimeValue::OptionSome(Box::new(decode_text(value)?)))
                }
            }
            (Self::OptionBytes, AbiValue::Pointer(value)) => {
                if value.is_null() {
                    Some(RuntimeValue::OptionNone)
                } else {
                    Some(RuntimeValue::OptionSome(Box::new(RuntimeValue::Bytes(
                        decode_len_prefixed_bytes(value)?,
                    ))))
                }
            }
            _ => None,
        }
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

fn write_pointer_cell(cell: &mut [u8], pointer: *const c_void) -> bool {
    let bytes = (pointer as usize).to_ne_bytes();
    if cell.len() < bytes.len() {
        return false;
    }
    cell[..bytes.len()].copy_from_slice(&bytes);
    true
}

fn record_call(profile: &mut EvaluationCallProfile, elapsed: Duration, cache_hit: bool) {
    profile.calls += 1;
    if cache_hit {
        profile.cache_hits += 1;
    }
    profile.total_time += elapsed;
    profile.max_time = profile.max_time.max(elapsed);
}
