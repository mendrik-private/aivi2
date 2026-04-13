use std::collections::BTreeMap;

use crate::{
    CompiledKernelArtifact, CompiledProgram, EvalFrame, EvaluationError, ItemId,
    KernelEvaluationProfile, KernelEvaluator, KernelFingerprint, KernelId, NativeKernelArtifactSet,
    Program, RuntimeValue,
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

    fn evaluate_signal_body_kernel(
        &mut self,
        kernel_id: KernelId,
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

/// Execution-time backend options that do not change backend IR or object emission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendExecutionOptions {
    /// Precompile dedicated signal body kernels when the JIT engine is created instead of waiting
    /// for the first call site to touch them.
    pub eagerly_compile_signals: bool,
    /// Force the backend to use the interpreter even when the executable program could create a
    /// lazy JIT engine.
    pub prefer_interpreter: bool,
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

    fn evaluate_signal_body_kernel(
        &mut self,
        kernel_id: KernelId,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        KernelEvaluator::evaluate_signal_body_kernel(self, kernel_id, environment, globals)
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
    native_kernels: Option<&'a NativeKernelArtifactSet>,
    execution_options: BackendExecutionOptions,
}

impl<'a> BackendExecutableProgram<'a> {
    /// Construct an executable program with no prebuilt object artifact.
    pub fn interpreted(program: &'a Program) -> Self {
        Self {
            program,
            compiled_object: None,
            native_kernels: None,
            execution_options: BackendExecutionOptions::default(),
        }
    }

    /// Attach an already-emitted object artifact while keeping lazy JIT execution available.
    pub fn from_compiled_object(program: &'a Program, compiled_object: CompiledProgram) -> Self {
        Self {
            program,
            compiled_object: Some(compiled_object),
            native_kernels: None,
            execution_options: BackendExecutionOptions::default(),
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

    pub fn execution_options(&self) -> BackendExecutionOptions {
        self.execution_options
    }

    pub fn with_execution_options(mut self, execution_options: BackendExecutionOptions) -> Self {
        self.execution_options = execution_options;
        self
    }

    pub fn with_native_kernels(mut self, native_kernels: &'a NativeKernelArtifactSet) -> Self {
        self.native_kernels = Some(native_kernels);
        self
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
        if self.execution_options.prefer_interpreter {
            BackendExecutionEngineKind::Interpreter
        } else {
            BackendExecutionEngineKind::Jit
        }
    }

    pub fn compiled_object(&self) -> Option<&CompiledProgram> {
        self.compiled_object.as_ref()
    }

    pub fn into_compiled_object(self) -> Option<CompiledProgram> {
        self.compiled_object
    }

    pub fn create_engine(&self) -> BackendExecutionEngineHandle<'a> {
        if self.execution_options.prefer_interpreter {
            Box::new(KernelEvaluator::new(self.program))
        } else if let Some(native_kernels) = self.native_kernels {
            Box::new(LazyJitExecutionEngine::new_with_native_artifacts(
                self.program,
                native_kernels,
                self.execution_options,
            ))
        } else {
            Box::new(LazyJitExecutionEngine::new(
                self.program,
                self.execution_options,
            ))
        }
    }

    pub fn create_profiled_engine(&self) -> BackendExecutionEngineHandle<'a> {
        if self.execution_options.prefer_interpreter {
            Box::new(KernelEvaluator::new_profiled(self.program))
        } else if let Some(native_kernels) = self.native_kernels {
            Box::new(LazyJitExecutionEngine::new_profiled_with_native_artifacts(
                self.program,
                native_kernels,
                self.execution_options,
            ))
        } else {
            Box::new(LazyJitExecutionEngine::new_profiled(
                self.program,
                self.execution_options,
            ))
        }
    }
}
