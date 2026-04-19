/// Stable content fingerprint for one backend kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct KernelFingerprint(u64);

impl KernelFingerprint {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for KernelFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Cranelift compilation results for one backend program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledProgram {
    object: Vec<u8>,
    kernels: Vec<CompiledKernel>,
    kernel_index: BTreeMap<KernelId, usize>,
}

impl CompiledProgram {
    /// Construct a `CompiledProgram` from raw object bytes and kernel metadata.
    /// Used by the artifact cache to reconstruct a cached program.
    pub fn new(object: Vec<u8>, kernels: Vec<CompiledKernel>) -> Self {
        let kernel_index = kernels
            .iter()
            .enumerate()
            .map(|(index, k)| (k.kernel, index))
            .collect();
        Self {
            object,
            kernels,
            kernel_index,
        }
    }

    pub fn object(&self) -> &[u8] {
        &self.object
    }

    pub fn kernels(&self) -> &[CompiledKernel] {
        &self.kernels
    }

    pub fn kernel(&self, id: KernelId) -> Option<&CompiledKernel> {
        self.kernel_index
            .get(&id)
            .and_then(|index| self.kernels.get(*index))
    }

    pub fn into_single_kernel_artifact(self) -> Option<CompiledKernelArtifact> {
        let CompiledProgram {
            object, kernels, ..
        } = self;
        let mut kernels = kernels.into_iter();
        let kernel = kernels.next()?;
        if kernels.next().is_some() {
            return None;
        }
        Some(CompiledKernelArtifact::new(object, kernel))
    }
}

/// Cranelift artifacts for one backend kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledKernel {
    pub kernel: KernelId,
    pub fingerprint: KernelFingerprint,
    pub symbol: Box<str>,
    pub clif: Box<str>,
    pub code_size: usize,
}

/// Self-contained object artifact for one compiled backend kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledKernelArtifact {
    object: Vec<u8>,
    metadata: CompiledKernel,
}

impl CompiledKernelArtifact {
    pub fn new(object: Vec<u8>, metadata: CompiledKernel) -> Self {
        Self { object, metadata }
    }

    pub fn object(&self) -> &[u8] {
        &self.object
    }

    pub fn metadata(&self) -> &CompiledKernel {
        &self.metadata
    }

    pub fn kernel_id(&self) -> KernelId {
        self.metadata.kernel
    }

    pub fn fingerprint(&self) -> KernelFingerprint {
        self.metadata.fingerprint
    }

    pub fn into_parts(self) -> (Vec<u8>, CompiledKernel) {
        (self.object, self.metadata)
    }
}

#[derive(Debug)]
pub(crate) struct JitDataSlot {
    pub(crate) item: ItemId,
    pub(crate) layout: LayoutId,
    pub(crate) cell: Box<[u8]>,
}

pub(crate) struct CompiledJitKernel {
    pub(crate) function: *const u8,
    pub(crate) caller: FunctionCaller,
    pub(crate) signal_slots: Vec<JitDataSlot>,
    pub(crate) imported_item_slots: Vec<JitDataSlot>,
    pub(crate) _module: JITModule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitKernelArtifact {
    pub(crate) requested_kernel: KernelId,
    pub(crate) kernels: Vec<CachedJitCompiledKernel>,
    pub(crate) signal_slots: Vec<CachedJitDataSlot>,
    pub(crate) imported_item_slots: Vec<CachedJitDataSlot>,
    pub(crate) callable_descriptors: Vec<CachedJitCallableDescriptor>,
    pub(crate) literal_data: Vec<CachedJitLiteralData>,
    pub(crate) external_funcs: Vec<Box<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitCompiledKernel {
    pub(crate) kernel: KernelId,
    pub(crate) bytes: Box<[u8]>,
    pub(crate) relocs: Vec<CachedJitReloc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitDataSlot {
    pub(crate) item: ItemId,
    pub(crate) layout: LayoutId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitCallableDescriptor {
    pub(crate) item: ItemId,
    pub(crate) body: KernelId,
    pub(crate) arity: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitLiteralData {
    pub(crate) symbol: Box<str>,
    pub(crate) align: u64,
    pub(crate) bytes: Box<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CachedJitReloc {
    pub(crate) offset: u32,
    pub(crate) kind: Reloc,
    pub(crate) target: CachedJitRelocTarget,
    pub(crate) addend: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedJitRelocTarget {
    Function(CachedJitFunctionTarget),
    FunctionOffset {
        target: CachedJitFunctionTarget,
        offset: u32,
    },
    Data(CachedJitDataTarget),
    LibCall(Box<str>),
    KnownSymbol(Box<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedJitFunctionTarget {
    Kernel(KernelId),
    External(Box<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedJitDataTarget {
    SignalSlot(ItemId),
    ImportedItemSlot(ItemId),
    CallableDescriptor(ItemId),
    Literal(Box<str>),
}

/// Intermediate result holding the built CLIF for one kernel, ready for parallel
/// Cranelift compilation. Produced by `build_kernel_clif`; consumed by `compile()`.
struct BuiltKernel {
    kernel_id: KernelId,
    func_id: FuncId,
    /// Built CLIF function context, ready for `ctx.compile(isa, ctrl_plane)`.
    ctx: cranelift_codegen::Context,
    /// Human-readable CLIF text snapshot taken before compilation.
    clif: Box<str>,
    symbol: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JitLiteralDataRecord {
    data_id: DataId,
    align: u64,
    bytes: Box<[u8]>,
}

pub type CodegenErrors = aivi_base::ErrorCollection<CodegenError>;
