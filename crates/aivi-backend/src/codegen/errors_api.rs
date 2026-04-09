#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    InvalidBackendProgram(ValidationError),
    MissingKernel {
        kernel: KernelId,
    },
    AmbientKernelUnsupported {
        kernel: KernelId,
    },
    HostIsaUnavailable {
        message: Box<str>,
    },
    TargetIsaCreation {
        message: Box<str>,
    },
    ObjectModuleCreation {
        message: Box<str>,
    },
    JitModuleCreation {
        message: Box<str>,
    },
    UnsupportedJitSymbol {
        kernel: KernelId,
        symbol: Box<str>,
    },
    UnsupportedKernelConvention {
        kernel: KernelId,
        kind: CallingConventionKind,
    },
    UnsupportedLayout {
        kernel: KernelId,
        layout: LayoutId,
        detail: Box<str>,
    },
    UnsupportedExpression {
        kernel: KernelId,
        expr: KernelExprId,
        detail: Box<str>,
    },
    MissingInputParameter {
        kernel: KernelId,
    },
    MissingEnvironmentParameter {
        kernel: KernelId,
        slot: EnvSlotId,
    },
    InvalidIntegerLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    InvalidFloatLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    InvalidDecimalLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    InvalidBigIntLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    CraneliftModule {
        kernel: Option<KernelId>,
        message: Box<str>,
    },
    CraneliftVerifier {
        kernel: KernelId,
        message: Box<str>,
    },
    ObjectEmission {
        message: Box<str>,
    },
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBackendProgram(error) => {
                write!(f, "Cranelift codegen requires valid backend IR: {error}")
            }
            Self::MissingKernel { kernel } => {
                write!(f, "backend program does not contain kernel {kernel}")
            }
            Self::AmbientKernelUnsupported { kernel } => write!(
                f,
                "kernel {kernel} is ambient runtime-only state and cannot be compiled into a standalone backend artifact"
            ),
            Self::HostIsaUnavailable { message } => {
                write!(
                    f,
                    "Cranelift codegen cannot target the current host ISA: {message}"
                )
            }
            Self::TargetIsaCreation { message } => {
                write!(
                    f,
                    "Cranelift codegen could not build the target ISA: {message}"
                )
            }
            Self::ObjectModuleCreation { message } => {
                write!(
                    f,
                    "Cranelift codegen could not create an object module: {message}"
                )
            }
            Self::JitModuleCreation { message } => {
                write!(
                    f,
                    "Cranelift codegen could not create a JIT module: {message}"
                )
            }
            Self::UnsupportedJitSymbol { kernel, symbol } => write!(
                f,
                "kernel {kernel} requires external symbol `{symbol}` that the lazy JIT engine cannot bind yet"
            ),
            Self::UnsupportedKernelConvention { kernel, kind } => {
                write!(
                    f,
                    "kernel {kernel} uses unsupported backend calling convention {kind}"
                )
            }
            Self::UnsupportedLayout {
                kernel,
                layout,
                detail,
            } => write!(
                f,
                "kernel {kernel} requires layout{layout} to stay in the backend/codegen layer: {detail}"
            ),
            Self::UnsupportedExpression {
                kernel,
                expr,
                detail,
            } => write!(
                f,
                "kernel {kernel} expression {expr} is outside the first Cranelift slice: {detail}"
            ),
            Self::MissingInputParameter { kernel } => write!(
                f,
                "kernel {kernel} references its input subject without a materialized Cranelift parameter"
            ),
            Self::MissingEnvironmentParameter { kernel, slot } => write!(
                f,
                "kernel {kernel} references environment slot {slot} without a materialized Cranelift parameter"
            ),
            Self::InvalidIntegerLiteral { kernel, expr, raw } => write!(
                f,
                "kernel {kernel} expression {expr} integer literal `{raw}` does not fit in the current i64 ABI slice"
            ),
            Self::InvalidFloatLiteral { kernel, expr, raw } => write!(
                f,
                "kernel {kernel} expression {expr} Float literal `{raw}` does not fit in the current finite f64 ABI slice"
            ),
            Self::InvalidDecimalLiteral { kernel, expr, raw } => write!(
                f,
                "kernel {kernel} expression {expr} Decimal literal `{raw}` does not fit the current backend decimal-literal cell format"
            ),
            Self::InvalidBigIntLiteral { kernel, expr, raw } => write!(
                f,
                "kernel {kernel} expression {expr} BigInt literal `{raw}` does not fit the current backend BigInt-literal cell format"
            ),
            Self::CraneliftModule {
                kernel: Some(kernel),
                message,
            } => write!(
                f,
                "Cranelift module failure while compiling kernel {kernel}: {message}"
            ),
            Self::CraneliftModule {
                kernel: None,
                message,
            } => write!(f, "Cranelift module failure: {message}"),
            Self::CraneliftVerifier { kernel, message } => {
                write!(f, "Cranelift verifier rejected kernel {kernel}: {message}")
            }
            Self::ObjectEmission { message } => {
                write!(f, "Cranelift object emission failed: {message}")
            }
        }
    }
}

impl std::error::Error for CodegenError {}

/// Lower one validated backend program into Cranelift functions and native object bytes.
///
/// The current slice is intentionally narrow:
/// - it consumes backend-owned ABI/layout contracts only,
/// - it maps `RuntimeKernelV1` onto the target's default call convention,
/// - it materializes `Int` as `i64`, `Float` as finite `f64`, `Bool` as `i8`, and backend
///   by-reference values as host pointers,
/// - it materializes `Decimal` / `BigInt` plus fragment-only `Text` literals as immutable
///   backend-owned constant cells behind those by-reference pointers,
/// - it materializes signal item reads as imported current-value slots in that same ABI shape,
/// - it materializes unsaturated top-level function items as local callable descriptor cells,
/// - it uses a backend-local pointer niche for `Option` over by-reference payloads,
/// - it resolves record projection offsets inside backend/codegen,
/// - it emits backend item-body kernels directly,
/// - it lowers saturated direct item calls, representational by-reference domain-member calls,
///   niche `Option` constructor calls already represented in backend IR,
/// - it lowers selected scalar unary/binary operators, including `Float` comparison/equality,
///   plus native equality for `Text`, record/tuple aggregates, and niche `Option` pointers whose
///   leaves are already codegen-supported, and
/// - it explicitly rejects the remaining apply/domain/builtin aggregate/collection/dynamic-text
///   lowering, plus inline-pipe `Case`/`TruthyFalsy`/`Debug` stages, until those contracts are
///   owned in this layer.
pub fn compile_program(program: &Program) -> Result<CompiledProgram, CodegenErrors> {
    validate_backend_program(program)?;
    let compiler = CraneliftCompiler::new(program).map_err(wrap_one)?;
    compiler.compile()
}

/// Compile a single backend kernel into a standalone object artifact while leaving interpreter
/// execution as the active runtime path.
pub fn compile_kernel(
    program: &Program,
    kernel_id: KernelId,
) -> Result<CompiledKernelArtifact, CodegenErrors> {
    validate_backend_program(program)?;
    let compiler = CraneliftCompiler::new(program).map_err(wrap_one)?;
    compiler.compile_kernel(kernel_id)
}

pub(crate) fn compile_kernel_jit(
    program: &Program,
    kernel_id: KernelId,
) -> Result<CompiledJitKernel, CodegenErrors> {
    compile_kernel_jit_with_cache_artifact(program, kernel_id).map(|(compiled, _)| compiled)
}

pub(crate) fn compile_kernel_jit_with_cache_artifact(
    program: &Program,
    kernel_id: KernelId,
) -> Result<(CompiledJitKernel, Option<CachedJitKernelArtifact>), CodegenErrors> {
    validate_backend_program(program)?;
    let compiler = CraneliftCompiler::new_jit(program).map_err(wrap_one)?;
    compiler.compile_kernel_jit_with_cache_artifact(kernel_id)
}

pub(crate) fn instantiate_cached_jit_kernel(
    program: &Program,
    kernel_id: KernelId,
    artifact: &CachedJitKernelArtifact,
) -> Result<CompiledJitKernel, CodegenErrors> {
    validate_backend_program(program)?;
    let compiler = CraneliftCompiler::new_jit(program).map_err(wrap_one)?;
    compiler.replay_cached_jit_kernel(kernel_id, artifact)
}

/// Stable symbol name for one backend kernel.
pub fn kernel_symbol(program: &Program, kernel_id: KernelId) -> String {
    let kernel = &program.kernels()[kernel_id];
    kernel_symbol_for(program, kernel_id, kernel)
}

/// Stable fingerprint for one backend kernel and its codegen-relevant dependencies.
pub fn compute_kernel_fingerprint(program: &Program, kernel_id: KernelId) -> KernelFingerprint {
    let kernel = &program.kernels()[kernel_id];
    compute_kernel_fingerprint_for(program, kernel_id, kernel)
}

fn jit_dependency_kernel_ids(
    program: &Program,
    kernel_id: KernelId,
) -> Result<Vec<KernelId>, CodegenError> {
    if program.kernels().get(kernel_id).is_none() {
        return Err(CodegenError::MissingKernel { kernel: kernel_id });
    }
    let mut kernels = BTreeSet::new();
    let mut seen_items = BTreeSet::new();
    collect_jit_kernel_dependencies(program, kernel_id, &mut kernels, &mut seen_items);
    Ok(kernels.into_iter().collect())
}

fn collect_jit_kernel_dependencies(
    program: &Program,
    kernel_id: KernelId,
    kernels: &mut BTreeSet<KernelId>,
    seen_items: &mut BTreeSet<ItemId>,
) {
    if !kernels.insert(kernel_id) {
        return;
    }
    let kernel = &program.kernels()[kernel_id];
    for (_, expr) in kernel.exprs().iter() {
        if let KernelExprKind::Item(item) = expr.kind {
            if !seen_items.insert(item) {
                continue;
            }
            let item_decl = &program.items()[item];
            if matches!(item_decl.kind, ItemKind::Signal(_)) {
                continue;
            }
            if let Some(body) = item_decl.body {
                collect_jit_kernel_dependencies(program, body, kernels, seen_items);
            }
        }
    }
}

struct CraneliftCompiler<'a, M: Module> {
    program: &'a Program,
    module: M,
    declared_functions: BTreeMap<KernelId, FuncId>,
    declared_signal_slots: BTreeMap<ItemId, DataId>,
    signal_slot_layouts: BTreeMap<ItemId, LayoutId>,
    declared_imported_item_slots: BTreeMap<ItemId, DataId>,
    imported_item_slot_layouts: BTreeMap<ItemId, LayoutId>,
    declared_callable_descriptors: BTreeMap<ItemId, DataId>,
    callable_descriptor_specs: BTreeMap<ItemId, (KernelId, usize)>,
    declared_external_funcs: BTreeMap<Box<str>, FuncId>,
    literal_data: BTreeMap<Box<str>, JitLiteralDataRecord>,
    function_builder_ctx: FunctionBuilderContext,
    next_data_symbol: u64,
    jit_symbols: Option<Arc<Mutex<BTreeMap<Box<str>, usize>>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DirectApplyPlan {
    Item {
        body: KernelId,
        arguments: Box<[(LayoutId, LayoutId)]>,
        result: (LayoutId, LayoutId),
    },
    ExternalItem {
        item: ItemId,
    },
    LocalFunctionAddress {
        body: KernelId,
    },
    DomainMember(DomainMemberCallPlan),
    Builtin(BuiltinCallPlan),
    Intrinsic(IntrinsicCallPlan),
    SumConstruction {
        variant_tag: i64,
        payload_layout: Option<LayoutId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemReferencePlan {
    DirectValue {
        body: KernelId,
        result: (LayoutId, LayoutId),
    },
    SignalSlot {
        item: ItemId,
    },
    ImportedSlot {
        item: ItemId,
    },
    CallableDescriptor {
        item: ItemId,
        body: KernelId,
        arity: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomainMemberCallPlan {
    RepresentationalIdentityUnary,
    /// `.carrier` where the carrier is a by-value primitive (e.g., `Duration over Int` → `Int`).
    CarrierExtractPrimitive,
    NativeIntBinary(BinaryOperator),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BuiltinCallPlan {
    OptionSome(OptionCodegenContract),
    ListReduce(ListReducePlan),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListReducePlan {
    step_body: KernelId,
    loop_layout: LayoutId,
    seed_layout: LayoutId,
    step_acc_layout: LayoutId,
    element_layout: LayoutId,
    step_element_layout: LayoutId,
    step_result_layout: LayoutId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntrinsicCallPlan {
    BytesLength,
    BytesGet,
    BytesFromText,
    BytesToText,
    BytesAppend,
    BytesRepeat,
    BytesSlice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarOptionKind {
    Int,
    Float,
    Bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptionCodegenContract {
    NicheReference,
    InlineScalar(ScalarOptionKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KernelLinkage {
    Local,
    Import,
}

impl KernelLinkage {
    const fn into_cranelift(self) -> Linkage {
        match self {
            Self::Local => Linkage::Local,
            Self::Import => Linkage::Import,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeCompareKind {
    Integer,
    Float,
    Decimal,
    BigInt,
    DomainInt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeArithmeticKind {
    Integer,
    Decimal,
    BigInt,
    DomainInt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeEqualityShape {
    Integer,
    Float,
    Decimal,
    BigInt,
    Text,
    Bytes,
    Aggregate(Vec<NativeEqualityField>),
    NicheOption { payload: Box<NativeEqualityShape> },
    InlineScalarOption(ScalarOptionKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeEqualityField {
    offset: i32,
    layout: LayoutId,
    shape: Box<NativeEqualityShape>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StaticMaterializationPlan {
    Int(i64),
    Float(RuntimeFloat),
    Bool(bool),
    Text(Box<str>),
    Bytes(Box<[u8]>),
    NicheOptionNone,
    NicheOptionSome(Box<StaticMaterializationPlan>),
    InlineScalarOptionNone(ScalarOptionKind),
    InlineScalarOptionSome {
        kind: ScalarOptionKind,
        payload: Box<StaticMaterializationPlan>,
    },
}

fn build_target_isa() -> Result<OwnedTargetIsa, CodegenError> {
    let isa_builder =
        cranelift_native::builder().map_err(|message| CodegenError::HostIsaUnavailable {
            message: message.to_owned().into_boxed_str(),
        })?;
    let mut flags = settings::builder();
    flags
        .enable("enable_llvm_abi_extensions")
        .map_err(|error| CodegenError::TargetIsaCreation {
            message: error.to_string().into_boxed_str(),
        })?;
    flags
        .set("opt_level", "speed")
        .map_err(|error| CodegenError::TargetIsaCreation {
            message: error.to_string().into_boxed_str(),
        })?;
    isa_builder
        .finish(settings::Flags::new(flags))
        .map_err(|error| CodegenError::TargetIsaCreation {
            message: error.to_string().into_boxed_str(),
        })
}

