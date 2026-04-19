use std::collections::BTreeMap;

use crate::{
    BackendRuntimeMeta, CallingConvention, CompiledKernelArtifact, CompiledProgram, EvalFrame,
    EvaluationError, Item, ItemId, KernelEvaluationProfile, KernelEvaluator, KernelFingerprint,
    KernelId, Layout, LayoutId, NativeKernelArtifactSet, Pipeline, Program, RuntimeValue, SourceId,
    SourcePlan,
    cache::{compile_kernel_cached, compile_program_cached},
    codegen::{CodegenErrors, compile_kernel, compile_program, compute_kernel_fingerprint},
    jit::{LazyJitExecutionEngine, NativeOnlyExecutionEngine},
    runtime::TaskFunctionApplier,
};

/// Stable backend execution surface shared by the interpreter fallback and lazy JIT engine.
pub trait BackendExecutionEngine: TaskFunctionApplier {
    fn kind(&self) -> BackendExecutionEngineKind;

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

#[derive(Clone, Copy, Debug)]
pub enum BackendRuntimeView<'a> {
    Program(&'a Program),
    Meta(&'a BackendRuntimeMeta),
}

#[derive(Clone, Copy, Debug)]
pub struct BackendRuntimeKernelRef<'a> {
    pub fingerprint: KernelFingerprint,
    pub input_subject: Option<LayoutId>,
    pub environment: &'a [LayoutId],
    pub result_layout: LayoutId,
    pub convention: &'a CallingConvention,
    pub global_items: &'a [ItemId],
}

impl<'a> BackendRuntimeView<'a> {
    pub fn item(self, item: ItemId) -> Option<&'a Item> {
        match self {
            Self::Program(program) => program.items().get(item),
            Self::Meta(meta) => meta.items().get(item),
        }
    }

    pub fn pipeline(self, pipeline: crate::PipelineId) -> Option<&'a Pipeline> {
        match self {
            Self::Program(program) => program.pipelines().get(pipeline),
            Self::Meta(meta) => meta.pipelines().get(pipeline),
        }
    }

    pub fn layout(self, layout: LayoutId) -> Option<&'a Layout> {
        match self {
            Self::Program(program) => program.layouts().get(layout),
            Self::Meta(meta) => meta.layouts().get(layout),
        }
    }

    pub fn source(self, source: SourceId) -> Option<&'a SourcePlan> {
        match self {
            Self::Program(program) => program.sources().get(source),
            Self::Meta(meta) => meta.sources().get(source),
        }
    }

    pub fn named_domain_carrier(self, layout: LayoutId) -> Option<LayoutId> {
        match self {
            Self::Program(program) => program.named_domain_carrier(layout),
            Self::Meta(meta) => meta.named_domain_carrier(layout),
        }
    }

    pub fn item_name(self, item: ItemId) -> Option<&'a str> {
        self.item(item).map(|item| item.name.as_ref())
    }

    pub fn kernel(self, kernel: KernelId) -> Option<BackendRuntimeKernelRef<'a>> {
        match self {
            Self::Program(program) => {
                program
                    .kernels()
                    .get(kernel)
                    .map(|kernel_meta| BackendRuntimeKernelRef {
                        fingerprint: compute_kernel_fingerprint(program, kernel),
                        input_subject: kernel_meta.input_subject,
                        environment: kernel_meta.environment.as_slice(),
                        result_layout: kernel_meta.result_layout,
                        convention: &kernel_meta.convention,
                        global_items: kernel_meta.global_items.as_slice(),
                    })
            }
            Self::Meta(meta) => {
                meta.kernels()
                    .get(kernel)
                    .map(|kernel_meta| BackendRuntimeKernelRef {
                        fingerprint: kernel_meta.fingerprint,
                        input_subject: kernel_meta.input_subject,
                        environment: kernel_meta.environment.as_slice(),
                        result_layout: kernel_meta.result_layout,
                        convention: &kernel_meta.convention,
                        global_items: kernel_meta.global_items.as_slice(),
                    })
            }
        }
    }

    pub fn as_program(self) -> Option<&'a Program> {
        match self {
            Self::Program(program) => Some(program),
            Self::Meta(_) => None,
        }
    }
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
    backend: BackendRuntimeView<'a>,
    compiled_object: Option<CompiledProgram>,
    native_kernels: Option<&'a NativeKernelArtifactSet>,
    execution_options: BackendExecutionOptions,
}

impl<'a> BackendExecutableProgram<'a> {
    /// Construct an executable program with no prebuilt object artifact.
    pub fn interpreted(program: &'a Program) -> Self {
        Self {
            backend: BackendRuntimeView::Program(program),
            compiled_object: None,
            native_kernels: None,
            execution_options: BackendExecutionOptions::default(),
        }
    }

    pub fn from_runtime_meta(meta: &'a BackendRuntimeMeta) -> Self {
        Self {
            backend: BackendRuntimeView::Meta(meta),
            compiled_object: None,
            native_kernels: None,
            execution_options: BackendExecutionOptions::default(),
        }
    }

    /// Attach an already-emitted object artifact while keeping lazy JIT execution available.
    pub fn from_compiled_object(program: &'a Program, compiled_object: CompiledProgram) -> Self {
        Self {
            backend: BackendRuntimeView::Program(program),
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

    pub fn backend(&self) -> BackendRuntimeView<'a> {
        self.backend
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
        compute_kernel_fingerprint(
            self.backend
                .as_program()
                .expect("kernel fingerprints require a full backend program"),
            kernel_id,
        )
    }

    pub fn compile_kernel(
        &self,
        kernel_id: KernelId,
    ) -> Result<CompiledKernelArtifact, CodegenErrors> {
        compile_kernel(
            self.backend
                .as_program()
                .expect("kernel compilation requires a full backend program"),
            kernel_id,
        )
    }

    pub fn compile_kernel_cached(
        &self,
        kernel_id: KernelId,
    ) -> Result<CompiledKernelArtifact, CodegenErrors> {
        compile_kernel_cached(
            self.backend
                .as_program()
                .expect("kernel compilation requires a full backend program"),
            kernel_id,
        )
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
        match self.backend {
            BackendRuntimeView::Program(program) => {
                if self.execution_options.prefer_interpreter {
                    Box::new(KernelEvaluator::new(program))
                } else if let Some(native_kernels) = self.native_kernels {
                    Box::new(LazyJitExecutionEngine::new_with_native_artifacts(
                        program,
                        native_kernels,
                        self.execution_options,
                    ))
                } else {
                    Box::new(LazyJitExecutionEngine::new(program, self.execution_options))
                }
            }
            BackendRuntimeView::Meta(meta) => Box::new(NativeOnlyExecutionEngine::new(
                meta,
                self.native_kernels,
                self.execution_options,
            )),
        }
    }

    pub fn create_profiled_engine(&self) -> BackendExecutionEngineHandle<'a> {
        match self.backend {
            BackendRuntimeView::Program(program) => {
                if self.execution_options.prefer_interpreter {
                    Box::new(KernelEvaluator::new_profiled(program))
                } else if let Some(native_kernels) = self.native_kernels {
                    Box::new(LazyJitExecutionEngine::new_profiled_with_native_artifacts(
                        program,
                        native_kernels,
                        self.execution_options,
                    ))
                } else {
                    Box::new(LazyJitExecutionEngine::new_profiled(
                        program,
                        self.execution_options,
                    ))
                }
            }
            BackendRuntimeView::Meta(meta) => Box::new(NativeOnlyExecutionEngine::new_profiled(
                meta,
                self.native_kernels,
                self.execution_options,
            )),
        }
    }
}
