use std::collections::BTreeMap;

use crate::{
    CompiledKernelArtifact, CompiledProgram, EvalFrame, EvaluationError, ItemId,
    KernelEvaluationProfile, KernelEvaluator, KernelFingerprint, KernelId, Program, RuntimeValue,
    cache::{compile_kernel_cached, compile_program_cached},
    codegen::{CodegenErrors, compile_kernel, compile_program, compute_kernel_fingerprint},
    jit::LazyJitExecutionEngine,
    runtime::TaskFunctionApplier,
};

/// Stable backend execution surface shared by the interpreter fallback and lazy JIT engine.
pub trait BackendExecutionEngine: TaskFunctionApplier {
    fn kind(&self) -> BackendExecutionEngineKind;

    fn program(&self) -> &Program;

    fn profile(&self) -> Option<&KernelEvaluationProfile>;

    fn profile_snapshot(&self) -> Option<KernelEvaluationProfile>;

    fn eval_trace(&self) -> &[EvalFrame];

    fn evaluate_kernel(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError>;

    fn apply_runtime_callable(
        &mut self,
        kernel_id: KernelId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError>;

    fn subtract_runtime_values(
        &self,
        kernel_id: KernelId,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError>;

    fn evaluate_item(
        &mut self,
        item: ItemId,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError>;
}

/// Boxed handle returned by [`BackendExecutableProgram`] when constructing an engine.
pub type BackendExecutionEngineHandle<'a> = Box<dyn BackendExecutionEngine + 'a>;

/// Runtime execution backends available to the backend layer.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendExecutionEngineKind {
    Interpreter,
    Jit,
}

impl BackendExecutionEngine for KernelEvaluator<'_> {
    fn kind(&self) -> BackendExecutionEngineKind {
        BackendExecutionEngineKind::Interpreter
    }

    fn program(&self) -> &Program {
        KernelEvaluator::program(self)
    }

    fn profile(&self) -> Option<&KernelEvaluationProfile> {
        KernelEvaluator::profile(self)
    }

    fn profile_snapshot(&self) -> Option<KernelEvaluationProfile> {
        KernelEvaluator::profile_snapshot(self)
    }

    fn eval_trace(&self) -> &[EvalFrame] {
        KernelEvaluator::eval_trace(self)
    }

    fn evaluate_kernel(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        KernelEvaluator::evaluate_kernel(self, kernel_id, input_subject, environment, globals)
    }

    fn apply_runtime_callable(
        &mut self,
        kernel_id: KernelId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        KernelEvaluator::apply_runtime_callable(self, kernel_id, callee, arguments, globals)
    }

    fn subtract_runtime_values(
        &self,
        kernel_id: KernelId,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        KernelEvaluator::subtract_runtime_values(self, kernel_id, left, right)
    }

    fn evaluate_item(
        &mut self,
        item: ItemId,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        KernelEvaluator::evaluate_item(self, item, globals)
    }
}

/// Backend-owned execution wrapper.
///
/// It preserves object-artifact compilation while routing live execution through the lazy JIT
/// engine, which falls back to `KernelEvaluator` when a kernel is outside the supported JIT slice.
#[derive(Clone, Debug)]
pub struct BackendExecutableProgram<'a> {
    program: &'a Program,
    compiled_object: Option<CompiledProgram>,
}

impl<'a> BackendExecutableProgram<'a> {
    /// Construct an executable program with no prebuilt object artifact.
    pub fn interpreted(program: &'a Program) -> Self {
        Self {
            program,
            compiled_object: None,
        }
    }

    /// Attach an already-emitted object artifact while keeping lazy JIT execution available.
    pub fn from_compiled_object(program: &'a Program, compiled_object: CompiledProgram) -> Self {
        Self {
            program,
            compiled_object: Some(compiled_object),
        }
    }

    /// Compile object code for the backend program while keeping lazy JIT execution as the active
    /// runtime path.
    pub fn compile(program: &'a Program) -> Result<Self, CodegenErrors> {
        compile_program(program)
            .map(|compiled_object| Self::from_compiled_object(program, compiled_object))
    }

    /// Compile or reuse a cached object artifact for the backend program while keeping lazy JIT
    /// execution as the active runtime path.
    pub fn compile_cached(program: &'a Program) -> Result<Self, CodegenErrors> {
        compile_program_cached(program)
            .map(|compiled_object| Self::from_compiled_object(program, compiled_object))
    }

    pub fn program(&self) -> &'a Program {
        self.program
    }

    pub fn kernel_fingerprint(&self, kernel_id: KernelId) -> KernelFingerprint {
        compute_kernel_fingerprint(self.program, kernel_id)
    }

    pub fn compile_kernel(
        &self,
        kernel_id: KernelId,
    ) -> Result<CompiledKernelArtifact, CodegenErrors> {
        compile_kernel(self.program, kernel_id)
    }

    pub fn compile_kernel_cached(
        &self,
        kernel_id: KernelId,
    ) -> Result<CompiledKernelArtifact, CodegenErrors> {
        compile_kernel_cached(self.program, kernel_id)
    }

    pub fn engine_kind(&self) -> BackendExecutionEngineKind {
        BackendExecutionEngineKind::Jit
    }

    pub fn compiled_object(&self) -> Option<&CompiledProgram> {
        self.compiled_object.as_ref()
    }

    pub fn into_compiled_object(self) -> Option<CompiledProgram> {
        self.compiled_object
    }

    pub fn create_engine(&self) -> BackendExecutionEngineHandle<'a> {
        Box::new(LazyJitExecutionEngine::new(self.program))
    }

    pub fn create_profiled_engine(&self) -> BackendExecutionEngineHandle<'a> {
        Box::new(LazyJitExecutionEngine::new_profiled(self.program))
    }
}
