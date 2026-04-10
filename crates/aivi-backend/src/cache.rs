//! Persistent cache surfaces for compiled backend artifacts.
//!
//! The backend keeps two related identities:
//! - a stable content fingerprint for a backend program or kernel, used by query/runtime layers to
//!   decide whether a compilation unit is semantically unchanged, and
//! - a disk-cache key that layers compiler-version and codegen-target namespace data on top of that
//!   stable fingerprint before reading or writing machine-code artifacts under XDG cache.
//!
//! Cache misses and corrupt entries are treated as non-fatal misses; the backend simply recompiles
//! and rewrites a fresh artifact.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    env, fs,
    hash::{Hash, Hasher},
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use cranelift_codegen::binemit::Reloc;
use rustc_hash::FxHasher;

use crate::{
    CodegenErrors, CompiledKernel, CompiledKernelArtifact, CompiledProgram, KernelFingerprint,
    KernelId,
    codegen::{
        CachedJitCallableDescriptor, CachedJitCompiledKernel, CachedJitDataSlot,
        CachedJitFunctionTarget, CachedJitKernelArtifact, CachedJitLiteralData, CachedJitReloc,
        CachedJitRelocTarget, compile_kernel, compile_kernel_jit_with_cache_artifact,
        compile_program, compute_kernel_fingerprint, instantiate_cached_jit_kernel,
    },
    program::Program,
};

/// Magic bytes: ASCII "AIVI" + format version byte.
const PROGRAM_CACHE_MAGIC: &[u8; 5] = b"AIVI\x02";
/// Magic bytes: ASCII "AIVK" + format version byte.
const KERNEL_CACHE_MAGIC: &[u8; 5] = b"AIVK\x01";
/// Magic bytes: ASCII "AIVJ" + format version byte.
const JIT_KERNEL_CACHE_MAGIC: &[u8; 5] = b"AIVJ\x01";

const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Bump when backend machine-code semantics change without a Cargo package-version change.
const CODEGEN_NAMESPACE_REVISION: &str = "4";
const SHARED_CODEGEN_SETTINGS: &[(&str, &str)] =
    &[("enable_llvm_abi_extensions", "1"), ("opt_level", "speed")];

/// In-memory cache for per-kernel object artifacts owned by the backend layer.
#[derive(Clone, Debug, Default)]
pub struct BackendKernelArtifactCache {
    artifacts: BTreeMap<ProgramKernelCacheKey, CompiledKernelArtifact>,
}

impl BackendKernelArtifactCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub fn get(
        &self,
        program: &Program,
        fingerprint: KernelFingerprint,
    ) -> Option<&CompiledKernelArtifact> {
        self.artifacts
            .get(&program_kernel_cache_key(program, fingerprint))
    }

    pub fn get_by_kernel(
        &self,
        program: &Program,
        kernel_id: KernelId,
    ) -> Option<&CompiledKernelArtifact> {
        if !program.kernels().contains(kernel_id) {
            return None;
        }
        self.get(program, compute_kernel_fingerprint(program, kernel_id))
    }

    pub fn insert(
        &mut self,
        program: &Program,
        artifact: CompiledKernelArtifact,
    ) -> Option<CompiledKernelArtifact> {
        self.artifacts.insert(
            program_kernel_cache_key(program, artifact.fingerprint()),
            artifact,
        )
    }

    pub fn get_or_compile(
        &mut self,
        program: &Program,
        kernel_id: KernelId,
    ) -> Result<&CompiledKernelArtifact, CodegenErrors> {
        if !program.kernels().contains(kernel_id) {
            let error = compile_kernel(program, kernel_id)
                .expect_err("compiling a missing kernel should produce a backend codegen error");
            return Err(error);
        }
        let fingerprint = compute_kernel_fingerprint(program, kernel_id);
        match self
            .artifacts
            .entry(program_kernel_cache_key(program, fingerprint))
        {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let artifact = compile_kernel(program, kernel_id)?;
                Ok(entry.insert(artifact))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ProgramKernelCacheKey {
    program_fingerprint: u64,
    kernel_fingerprint: KernelFingerprint,
}

fn program_kernel_cache_key(
    program: &Program,
    kernel_fingerprint: KernelFingerprint,
) -> ProgramKernelCacheKey {
    ProgramKernelCacheKey {
        program_fingerprint: compute_program_fingerprint(program),
        kernel_fingerprint,
    }
}

/// Compute a stable content fingerprint for one backend program.
pub fn compute_program_fingerprint(program: &Program) -> u64 {
    let mut hasher = FxHasher::default();
    format!("{program:?}").hash(&mut hasher);
    hasher.finish()
}

/// Compute a stable 64-bit disk-cache key by layering compiler/codegen namespace
/// identity over a stable backend-program fingerprint.
pub fn compute_program_cache_key_from_fingerprint(fingerprint: u64) -> u64 {
    fingerprint ^ cache_namespace_hash().rotate_left(32)
}

/// Compute a stable 64-bit cache key for a backend program.
pub fn compute_program_cache_key(program: &Program) -> u64 {
    compute_program_cache_key_from_fingerprint(compute_program_fingerprint(program))
}

/// Compute a stable 64-bit cache key from a kernel fingerprint alone.
///
/// Full on-disk kernel artifact caches additionally scope this by the enclosing
/// backend program fingerprint so changed helper bodies cannot alias one another
/// across different programs.
pub fn compute_kernel_cache_key(fingerprint: KernelFingerprint) -> u64 {
    compute_program_cache_key_from_fingerprint(fingerprint.as_raw())
}

fn compute_program_scoped_kernel_cache_key(
    program: &Program,
    fingerprint: KernelFingerprint,
) -> u64 {
    let mut hasher = FxHasher::default();
    program_kernel_cache_key(program, fingerprint).hash(&mut hasher);
    compute_program_cache_key_from_fingerprint(hasher.finish())
}

fn compiler_version_hash() -> u64 {
    let mut version_hasher = FxHasher::default();
    COMPILER_VERSION.hash(&mut version_hasher);
    version_hasher.finish()
}

fn cache_namespace_hash() -> u64 {
    let mut hasher = FxHasher::default();
    compiler_version_hash().hash(&mut hasher);
    CODEGEN_NAMESPACE_REVISION.hash(&mut hasher);
    native_codegen_target_identity().hash(&mut hasher);
    for (name, value) in SHARED_CODEGEN_SETTINGS {
        name.hash(&mut hasher);
        value.hash(&mut hasher);
    }
    hasher.finish()
}

fn native_codegen_target_identity() -> String {
    cranelift_native::builder()
        .map(|builder| builder.triple().to_string())
        .unwrap_or_else(|_| {
            format!(
                "{}-{}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS,
                std::env::consts::FAMILY
            )
        })
}

fn cache_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("aivi").join("compiled"))
}

fn program_cache_path_in(cache_root: &Path, key: u64) -> PathBuf {
    cache_root.join(format!("program-{key:016x}.bin"))
}

fn kernel_cache_path_in(cache_root: &Path, key: u64) -> PathBuf {
    cache_root.join("kernels").join(format!("{key:016x}.bin"))
}

fn jit_kernel_cache_path_in(cache_root: &Path, key: u64) -> PathBuf {
    cache_root
        .join("jit-kernels")
        .join(format!("{key:016x}.bin"))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Option<u64> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Option<u8> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf).ok()?;
    Some(buf[0])
}

fn read_boxed_str(cursor: &mut Cursor<&[u8]>) -> Option<Box<str>> {
    let len = read_u32(cursor)? as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok().map(String::into_boxed_str)
}

fn read_boxed_bytes(cursor: &mut Cursor<&[u8]>) -> Option<Box<[u8]>> {
    let len = read_u64(cursor)? as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf).ok()?;
    Some(buf.into_boxed_slice())
}

fn write_boxed_str(buf: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn write_boxed_bytes(buf: &mut Vec<u8>, value: &[u8]) {
    buf.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buf.extend_from_slice(value);
}

fn serialize_compiled_kernel(buf: &mut Vec<u8>, kernel: &CompiledKernel) {
    buf.extend_from_slice(&kernel.kernel.as_raw().to_le_bytes());
    buf.extend_from_slice(&kernel.fingerprint.as_raw().to_le_bytes());

    let symbol = kernel.symbol.as_bytes();
    buf.extend_from_slice(&(symbol.len() as u32).to_le_bytes());
    buf.extend_from_slice(symbol);

    let clif = kernel.clif.as_bytes();
    buf.extend_from_slice(&(clif.len() as u32).to_le_bytes());
    buf.extend_from_slice(clif);

    buf.extend_from_slice(&(kernel.code_size as u64).to_le_bytes());
}

fn deserialize_compiled_kernel(cursor: &mut Cursor<&[u8]>) -> Option<CompiledKernel> {
    let kernel_raw = read_u32(cursor)?;
    let fingerprint = KernelFingerprint::new(read_u64(cursor)?);
    let symbol = read_boxed_str(cursor)?;
    let clif = read_boxed_str(cursor)?;
    let code_size = read_u64(cursor)? as usize;
    Some(CompiledKernel {
        kernel: KernelId::from_raw(kernel_raw),
        fingerprint,
        symbol,
        clif,
        code_size,
    })
}

fn serialize_program(compiled: &CompiledProgram) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(PROGRAM_CACHE_MAGIC);

    let object = compiled.object();
    buf.extend_from_slice(&(object.len() as u64).to_le_bytes());
    buf.extend_from_slice(object);

    let kernels = compiled.kernels();
    buf.extend_from_slice(&(kernels.len() as u32).to_le_bytes());
    for kernel in kernels {
        serialize_compiled_kernel(&mut buf, kernel);
    }
    buf
}

fn deserialize_program(bytes: &[u8]) -> Option<CompiledProgram> {
    let mut cursor = Cursor::new(bytes);

    let mut magic = [0u8; 5];
    cursor.read_exact(&mut magic).ok()?;
    if &magic != PROGRAM_CACHE_MAGIC {
        return None;
    }

    let object_len = read_u64(&mut cursor)? as usize;
    let mut object = vec![0u8; object_len];
    cursor.read_exact(&mut object).ok()?;

    let kernel_count = read_u32(&mut cursor)? as usize;
    let mut kernels = Vec::with_capacity(kernel_count);
    for _ in 0..kernel_count {
        kernels.push(deserialize_compiled_kernel(&mut cursor)?);
    }

    Some(CompiledProgram::new(object, kernels))
}

fn serialize_kernel_artifact(artifact: &CompiledKernelArtifact) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(KERNEL_CACHE_MAGIC);

    let object = artifact.object();
    buf.extend_from_slice(&(object.len() as u64).to_le_bytes());
    buf.extend_from_slice(object);
    serialize_compiled_kernel(&mut buf, artifact.metadata());
    buf
}

fn deserialize_kernel_artifact(bytes: &[u8]) -> Option<CompiledKernelArtifact> {
    let mut cursor = Cursor::new(bytes);

    let mut magic = [0u8; 5];
    cursor.read_exact(&mut magic).ok()?;
    if &magic != KERNEL_CACHE_MAGIC {
        return None;
    }

    let object_len = read_u64(&mut cursor)? as usize;
    let mut object = vec![0u8; object_len];
    cursor.read_exact(&mut object).ok()?;
    let metadata = deserialize_compiled_kernel(&mut cursor)?;
    Some(CompiledKernelArtifact::new(object, metadata))
}

fn serialize_reloc_kind(buf: &mut Vec<u8>, reloc: Reloc) {
    write_boxed_str(buf, &format!("{reloc:?}"));
}

fn deserialize_reloc_kind(cursor: &mut Cursor<&[u8]>) -> Option<Reloc> {
    match read_boxed_str(cursor)?.as_ref() {
        "Abs4" => Some(Reloc::Abs4),
        "Abs8" => Some(Reloc::Abs8),
        "X86PCRel4" => Some(Reloc::X86PCRel4),
        "X86CallPCRel4" => Some(Reloc::X86CallPCRel4),
        "X86CallPLTRel4" => Some(Reloc::X86CallPLTRel4),
        "X86GOTPCRel4" => Some(Reloc::X86GOTPCRel4),
        "X86SecRel" => Some(Reloc::X86SecRel),
        "Arm32Call" => Some(Reloc::Arm32Call),
        "Arm64Call" => Some(Reloc::Arm64Call),
        "S390xPCRel32Dbl" => Some(Reloc::S390xPCRel32Dbl),
        "S390xPLTRel32Dbl" => Some(Reloc::S390xPLTRel32Dbl),
        "ElfX86_64TlsGd" => Some(Reloc::ElfX86_64TlsGd),
        "MachOX86_64Tlv" => Some(Reloc::MachOX86_64Tlv),
        "MachOAarch64TlsAdrPage21" => Some(Reloc::MachOAarch64TlsAdrPage21),
        "MachOAarch64TlsAdrPageOff12" => Some(Reloc::MachOAarch64TlsAdrPageOff12),
        "Aarch64TlsDescAdrPage21" => Some(Reloc::Aarch64TlsDescAdrPage21),
        "Aarch64TlsDescLd64Lo12" => Some(Reloc::Aarch64TlsDescLd64Lo12),
        "Aarch64TlsDescAddLo12" => Some(Reloc::Aarch64TlsDescAddLo12),
        "Aarch64TlsDescCall" => Some(Reloc::Aarch64TlsDescCall),
        "Aarch64AdrGotPage21" => Some(Reloc::Aarch64AdrGotPage21),
        "Aarch64AdrPrelPgHi21" => Some(Reloc::Aarch64AdrPrelPgHi21),
        "Aarch64AddAbsLo12Nc" => Some(Reloc::Aarch64AddAbsLo12Nc),
        "Aarch64Ld64GotLo12Nc" => Some(Reloc::Aarch64Ld64GotLo12Nc),
        "RiscvCallPlt" => Some(Reloc::RiscvCallPlt),
        "RiscvTlsGdHi20" => Some(Reloc::RiscvTlsGdHi20),
        "RiscvPCRelLo12I" => Some(Reloc::RiscvPCRelLo12I),
        "RiscvGotHi20" => Some(Reloc::RiscvGotHi20),
        "RiscvPCRelHi20" => Some(Reloc::RiscvPCRelHi20),
        "S390xTlsGd64" => Some(Reloc::S390xTlsGd64),
        "S390xTlsGdCall" => Some(Reloc::S390xTlsGdCall),
        "PulleyPcRel" => Some(Reloc::PulleyPcRel),
        "PulleyCallIndirectHost" => Some(Reloc::PulleyCallIndirectHost),
        _ => None,
    }
}

fn serialize_cached_jit_function_target(buf: &mut Vec<u8>, target: &CachedJitFunctionTarget) {
    match target {
        CachedJitFunctionTarget::Kernel(kernel) => {
            buf.push(0);
            buf.extend_from_slice(&kernel.as_raw().to_le_bytes());
        }
        CachedJitFunctionTarget::External(symbol) => {
            buf.push(1);
            write_boxed_str(buf, symbol);
        }
    }
}

fn deserialize_cached_jit_function_target(
    cursor: &mut Cursor<&[u8]>,
) -> Option<CachedJitFunctionTarget> {
    match read_u8(cursor)? {
        0 => Some(CachedJitFunctionTarget::Kernel(KernelId::from_raw(
            read_u32(cursor)?,
        ))),
        1 => Some(CachedJitFunctionTarget::External(read_boxed_str(cursor)?)),
        _ => None,
    }
}

fn serialize_cached_jit_data_target(
    buf: &mut Vec<u8>,
    target: &crate::codegen::CachedJitDataTarget,
) {
    match target {
        crate::codegen::CachedJitDataTarget::SignalSlot(item) => {
            buf.push(0);
            buf.extend_from_slice(&item.as_raw().to_le_bytes());
        }
        crate::codegen::CachedJitDataTarget::ImportedItemSlot(item) => {
            buf.push(1);
            buf.extend_from_slice(&item.as_raw().to_le_bytes());
        }
        crate::codegen::CachedJitDataTarget::CallableDescriptor(item) => {
            buf.push(2);
            buf.extend_from_slice(&item.as_raw().to_le_bytes());
        }
        crate::codegen::CachedJitDataTarget::Literal(symbol) => {
            buf.push(3);
            write_boxed_str(buf, symbol);
        }
    }
}

fn deserialize_cached_jit_data_target(
    cursor: &mut Cursor<&[u8]>,
) -> Option<crate::codegen::CachedJitDataTarget> {
    match read_u8(cursor)? {
        0 => Some(crate::codegen::CachedJitDataTarget::SignalSlot(
            crate::ItemId::from_raw(read_u32(cursor)?),
        )),
        1 => Some(crate::codegen::CachedJitDataTarget::ImportedItemSlot(
            crate::ItemId::from_raw(read_u32(cursor)?),
        )),
        2 => Some(crate::codegen::CachedJitDataTarget::CallableDescriptor(
            crate::ItemId::from_raw(read_u32(cursor)?),
        )),
        3 => Some(crate::codegen::CachedJitDataTarget::Literal(
            read_boxed_str(cursor)?,
        )),
        _ => None,
    }
}

fn serialize_cached_jit_reloc_target(buf: &mut Vec<u8>, target: &CachedJitRelocTarget) {
    match target {
        CachedJitRelocTarget::Function(target) => {
            buf.push(0);
            serialize_cached_jit_function_target(buf, target);
        }
        CachedJitRelocTarget::FunctionOffset { target, offset } => {
            buf.push(1);
            serialize_cached_jit_function_target(buf, target);
            buf.extend_from_slice(&offset.to_le_bytes());
        }
        CachedJitRelocTarget::Data(target) => {
            buf.push(2);
            serialize_cached_jit_data_target(buf, target);
        }
        CachedJitRelocTarget::LibCall(symbol) => {
            buf.push(3);
            write_boxed_str(buf, symbol);
        }
        CachedJitRelocTarget::KnownSymbol(symbol) => {
            buf.push(4);
            write_boxed_str(buf, symbol);
        }
    }
}

fn deserialize_cached_jit_reloc_target(cursor: &mut Cursor<&[u8]>) -> Option<CachedJitRelocTarget> {
    match read_u8(cursor)? {
        0 => Some(CachedJitRelocTarget::Function(
            deserialize_cached_jit_function_target(cursor)?,
        )),
        1 => Some(CachedJitRelocTarget::FunctionOffset {
            target: deserialize_cached_jit_function_target(cursor)?,
            offset: read_u32(cursor)?,
        }),
        2 => Some(CachedJitRelocTarget::Data(
            deserialize_cached_jit_data_target(cursor)?,
        )),
        3 => Some(CachedJitRelocTarget::LibCall(read_boxed_str(cursor)?)),
        4 => Some(CachedJitRelocTarget::KnownSymbol(read_boxed_str(cursor)?)),
        _ => None,
    }
}

fn serialize_cached_jit_reloc(buf: &mut Vec<u8>, reloc: &CachedJitReloc) {
    buf.extend_from_slice(&reloc.offset.to_le_bytes());
    serialize_reloc_kind(buf, reloc.kind);
    serialize_cached_jit_reloc_target(buf, &reloc.target);
    buf.extend_from_slice(&reloc.addend.to_le_bytes());
}

fn deserialize_cached_jit_reloc(cursor: &mut Cursor<&[u8]>) -> Option<CachedJitReloc> {
    Some(CachedJitReloc {
        offset: read_u32(cursor)?,
        kind: deserialize_reloc_kind(cursor)?,
        target: deserialize_cached_jit_reloc_target(cursor)?,
        addend: {
            let mut buf = [0u8; 8];
            cursor.read_exact(&mut buf).ok()?;
            i64::from_le_bytes(buf)
        },
    })
}

fn serialize_cached_jit_kernel(buf: &mut Vec<u8>, kernel: &CachedJitCompiledKernel) {
    buf.extend_from_slice(&kernel.kernel.as_raw().to_le_bytes());
    write_boxed_bytes(buf, &kernel.bytes);
    buf.extend_from_slice(&(kernel.relocs.len() as u32).to_le_bytes());
    for reloc in &kernel.relocs {
        serialize_cached_jit_reloc(buf, reloc);
    }
}

fn deserialize_cached_jit_kernel(cursor: &mut Cursor<&[u8]>) -> Option<CachedJitCompiledKernel> {
    let kernel = KernelId::from_raw(read_u32(cursor)?);
    let bytes = read_boxed_bytes(cursor)?;
    let reloc_count = read_u32(cursor)? as usize;
    let mut relocs = Vec::with_capacity(reloc_count);
    for _ in 0..reloc_count {
        relocs.push(deserialize_cached_jit_reloc(cursor)?);
    }
    Some(CachedJitCompiledKernel {
        kernel,
        bytes,
        relocs,
    })
}

fn serialize_cached_jit_artifact(artifact: &CachedJitKernelArtifact) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(JIT_KERNEL_CACHE_MAGIC);
    buf.extend_from_slice(&artifact.requested_kernel.as_raw().to_le_bytes());

    buf.extend_from_slice(&(artifact.kernels.len() as u32).to_le_bytes());
    for kernel in &artifact.kernels {
        serialize_cached_jit_kernel(&mut buf, kernel);
    }

    buf.extend_from_slice(&(artifact.signal_slots.len() as u32).to_le_bytes());
    for slot in &artifact.signal_slots {
        buf.extend_from_slice(&slot.item.as_raw().to_le_bytes());
        buf.extend_from_slice(&slot.layout.as_raw().to_le_bytes());
    }

    buf.extend_from_slice(&(artifact.imported_item_slots.len() as u32).to_le_bytes());
    for slot in &artifact.imported_item_slots {
        buf.extend_from_slice(&slot.item.as_raw().to_le_bytes());
        buf.extend_from_slice(&slot.layout.as_raw().to_le_bytes());
    }

    buf.extend_from_slice(&(artifact.callable_descriptors.len() as u32).to_le_bytes());
    for descriptor in &artifact.callable_descriptors {
        buf.extend_from_slice(&descriptor.item.as_raw().to_le_bytes());
        buf.extend_from_slice(&descriptor.body.as_raw().to_le_bytes());
        buf.extend_from_slice(&descriptor.arity.to_le_bytes());
    }

    buf.extend_from_slice(&(artifact.literal_data.len() as u32).to_le_bytes());
    for literal in &artifact.literal_data {
        write_boxed_str(&mut buf, &literal.symbol);
        buf.extend_from_slice(&literal.align.to_le_bytes());
        write_boxed_bytes(&mut buf, &literal.bytes);
    }

    buf.extend_from_slice(&(artifact.external_funcs.len() as u32).to_le_bytes());
    for symbol in &artifact.external_funcs {
        write_boxed_str(&mut buf, symbol);
    }

    buf
}

fn deserialize_cached_jit_artifact(bytes: &[u8]) -> Option<CachedJitKernelArtifact> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 5];
    cursor.read_exact(&mut magic).ok()?;
    if &magic != JIT_KERNEL_CACHE_MAGIC {
        return None;
    }

    let requested_kernel = KernelId::from_raw(read_u32(&mut cursor)?);

    let kernel_count = read_u32(&mut cursor)? as usize;
    let mut kernels = Vec::with_capacity(kernel_count);
    for _ in 0..kernel_count {
        kernels.push(deserialize_cached_jit_kernel(&mut cursor)?);
    }

    let signal_count = read_u32(&mut cursor)? as usize;
    let mut signal_slots = Vec::with_capacity(signal_count);
    for _ in 0..signal_count {
        signal_slots.push(CachedJitDataSlot {
            item: crate::ItemId::from_raw(read_u32(&mut cursor)?),
            layout: crate::LayoutId::from_raw(read_u32(&mut cursor)?),
        });
    }

    let imported_count = read_u32(&mut cursor)? as usize;
    let mut imported_item_slots = Vec::with_capacity(imported_count);
    for _ in 0..imported_count {
        imported_item_slots.push(CachedJitDataSlot {
            item: crate::ItemId::from_raw(read_u32(&mut cursor)?),
            layout: crate::LayoutId::from_raw(read_u32(&mut cursor)?),
        });
    }

    let descriptor_count = read_u32(&mut cursor)? as usize;
    let mut callable_descriptors = Vec::with_capacity(descriptor_count);
    for _ in 0..descriptor_count {
        callable_descriptors.push(CachedJitCallableDescriptor {
            item: crate::ItemId::from_raw(read_u32(&mut cursor)?),
            body: KernelId::from_raw(read_u32(&mut cursor)?),
            arity: read_u32(&mut cursor)?,
        });
    }

    let literal_count = read_u32(&mut cursor)? as usize;
    let mut literal_data = Vec::with_capacity(literal_count);
    for _ in 0..literal_count {
        literal_data.push(CachedJitLiteralData {
            symbol: read_boxed_str(&mut cursor)?,
            align: read_u64(&mut cursor)?,
            bytes: read_boxed_bytes(&mut cursor)?,
        });
    }

    let external_count = read_u32(&mut cursor)? as usize;
    let mut external_funcs = Vec::with_capacity(external_count);
    for _ in 0..external_count {
        external_funcs.push(read_boxed_str(&mut cursor)?);
    }

    Some(CachedJitKernelArtifact {
        requested_kernel,
        kernels,
        signal_slots,
        imported_item_slots,
        callable_descriptors,
        literal_data,
        external_funcs,
    })
}

/// Load a cached `CompiledProgram` for the given key, if a valid entry exists.
pub fn load_cached_program(key: u64) -> Option<CompiledProgram> {
    let cache_root = cache_dir()?;
    load_cached_program_from(&cache_root, key)
}

fn load_cached_program_from(cache_root: &Path, key: u64) -> Option<CompiledProgram> {
    let path = program_cache_path_in(cache_root, key);
    let bytes = fs::read(&path).ok()?;
    deserialize_program(&bytes)
}

/// Persist a `CompiledProgram` to the disk cache under the given key.
/// Silently ignores I/O failures so a missing or read-only cache never breaks compilation.
pub fn store_cached_program(key: u64, compiled: &CompiledProgram) {
    let Some(cache_root) = cache_dir() else {
        return;
    };
    store_cached_program_in(&cache_root, key, compiled);
}

fn store_cached_program_in(cache_root: &Path, key: u64, compiled: &CompiledProgram) {
    let path = program_cache_path_in(cache_root, key);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize_program(compiled));
}

/// Load a cached per-kernel object artifact, if a valid entry exists.
pub fn load_cached_kernel_artifact(
    program: &Program,
    fingerprint: KernelFingerprint,
) -> Option<CompiledKernelArtifact> {
    let cache_root = cache_dir()?;
    load_cached_kernel_artifact_from(&cache_root, program, fingerprint)
}

fn load_cached_kernel_artifact_from(
    cache_root: &Path,
    program: &Program,
    fingerprint: KernelFingerprint,
) -> Option<CompiledKernelArtifact> {
    let path = kernel_cache_path_in(
        cache_root,
        compute_program_scoped_kernel_cache_key(program, fingerprint),
    );
    let bytes = fs::read(&path).ok()?;
    let artifact = deserialize_kernel_artifact(&bytes)?;
    (artifact.fingerprint() == fingerprint).then_some(artifact)
}

fn load_cached_jit_kernel_artifact_from(
    cache_root: &Path,
    program: &Program,
    fingerprint: KernelFingerprint,
) -> Option<CachedJitKernelArtifact> {
    let path = jit_kernel_cache_path_in(
        cache_root,
        compute_program_scoped_kernel_cache_key(program, fingerprint),
    );
    let bytes = fs::read(&path).ok()?;
    deserialize_cached_jit_artifact(&bytes)
}

/// Persist a per-kernel object artifact to the disk cache.
/// Silently ignores I/O failures so a missing or read-only cache never breaks compilation.
pub fn store_cached_kernel_artifact(
    program: &Program,
    fingerprint: KernelFingerprint,
    artifact: &CompiledKernelArtifact,
) {
    if artifact.fingerprint() != fingerprint {
        return;
    }
    let Some(cache_root) = cache_dir() else {
        return;
    };
    store_cached_kernel_artifact_in(&cache_root, program, fingerprint, artifact);
}

fn store_cached_kernel_artifact_in(
    cache_root: &Path,
    program: &Program,
    fingerprint: KernelFingerprint,
    artifact: &CompiledKernelArtifact,
) {
    let path = kernel_cache_path_in(
        cache_root,
        compute_program_scoped_kernel_cache_key(program, fingerprint),
    );
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize_kernel_artifact(artifact));
}

fn store_cached_jit_kernel_artifact_in(
    cache_root: &Path,
    program: &Program,
    fingerprint: KernelFingerprint,
    artifact: &CachedJitKernelArtifact,
) {
    let path = jit_kernel_cache_path_in(
        cache_root,
        compute_program_scoped_kernel_cache_key(program, fingerprint),
    );
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize_cached_jit_artifact(artifact));
}

/// Compile a backend program, consulting the disk cache first to skip Cranelift
/// codegen for unchanged programs. Falls back to full compilation on cache miss
/// or any deserialization error.
pub fn compile_program_cached(program: &Program) -> Result<CompiledProgram, CodegenErrors> {
    let Some(cache_root) = cache_dir() else {
        return compile_program(program);
    };
    compile_program_cached_in_dir(&cache_root, program)
}

fn compile_program_cached_in_dir(
    cache_root: &Path,
    program: &Program,
) -> Result<CompiledProgram, CodegenErrors> {
    let key = compute_program_cache_key(program);
    if let Some(cached) = load_cached_program_from(cache_root, key) {
        return Ok(cached);
    }
    let compiled = compile_program(program)?;
    store_cached_program_in(cache_root, key, &compiled);
    Ok(compiled)
}

/// Compile one backend kernel, consulting the disk cache first to skip Cranelift codegen for
/// unchanged per-kernel artifacts.
pub fn compile_kernel_cached(
    program: &Program,
    kernel_id: KernelId,
) -> Result<CompiledKernelArtifact, CodegenErrors> {
    if !program.kernels().contains(kernel_id) {
        return compile_kernel(program, kernel_id);
    }
    let Some(cache_root) = cache_dir() else {
        return compile_kernel(program, kernel_id);
    };
    compile_kernel_cached_in_dir(&cache_root, program, kernel_id)
}

fn compile_kernel_cached_in_dir(
    cache_root: &Path,
    program: &Program,
    kernel_id: KernelId,
) -> Result<CompiledKernelArtifact, CodegenErrors> {
    let fingerprint = compute_kernel_fingerprint(program, kernel_id);
    if let Some(cached) = load_cached_kernel_artifact_from(cache_root, program, fingerprint) {
        return Ok(cached);
    }
    let compiled = compile_kernel(program, kernel_id)?;
    store_cached_kernel_artifact_in(cache_root, program, fingerprint, &compiled);
    Ok(compiled)
}

pub(crate) fn compile_kernel_jit_cached(
    program: &Program,
    kernel_id: KernelId,
) -> Result<crate::codegen::CompiledJitKernel, CodegenErrors> {
    if !program.kernels().contains(kernel_id) {
        return crate::codegen::compile_kernel_jit(program, kernel_id);
    }
    let Some(cache_root) = cache_dir() else {
        return crate::codegen::compile_kernel_jit(program, kernel_id);
    };
    compile_kernel_jit_cached_in_dir(&cache_root, program, kernel_id)
}

fn compile_kernel_jit_cached_in_dir(
    cache_root: &Path,
    program: &Program,
    kernel_id: KernelId,
) -> Result<crate::codegen::CompiledJitKernel, CodegenErrors> {
    let fingerprint = compute_kernel_fingerprint(program, kernel_id);
    if let Some(cached) = load_cached_jit_kernel_artifact_from(cache_root, program, fingerprint)
        && let Ok(compiled) = instantiate_cached_jit_kernel(program, kernel_id, &cached)
    {
        return Ok(compiled);
    }
    let (compiled, artifact) = compile_kernel_jit_with_cache_artifact(program, kernel_id)?;
    if let Some(artifact) = artifact.as_ref() {
        store_cached_jit_kernel_artifact_in(cache_root, program, fingerprint, artifact);
    }
    Ok(compiled)
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        fs,
        rc::Rc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use aivi_base::SourceDatabase;
    use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
    use aivi_ffi_call::{
        AbiValue, AllocationArena, FunctionCaller, decode_len_prefixed_bytes, decode_marshaled_map,
        decode_marshaled_sequence, read_bigint_constant_bytes, read_decimal_constant_bytes,
        with_active_arena,
    };
    use aivi_lambda::{
        lower_module as lower_lambda_module, validate_module as validate_lambda_module,
    };
    use aivi_syntax::parse_module;

    use super::*;
    use crate::{
        RuntimeBigInt, RuntimeDecimal, lower_module as lower_backend_module, validate_program,
    };

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

    fn find_item(program: &Program, name: &str) -> crate::ItemId {
        program
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("expected backend item `{name}`"))
    }

    fn with_temp_cache_dir<R>(f: impl FnOnce(&Path) -> R) -> R {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "aivi-backend-cache-test-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp cache root should be created");
        let result = f(&dir);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn program_cache_key_reuses_stable_fingerprint_with_namespace_layering() {
        let backend = lower_text(
            "cache-program-fingerprint.aivi",
            "value total:Int = 21 + 21\nvalue other:Int = 1 + 1\n",
        );
        let changed = lower_text(
            "cache-program-fingerprint.aivi",
            "value total:Int = 21 + 21\nvalue other:Int = 2 + 2\n",
        );

        let fingerprint = compute_program_fingerprint(&backend);

        assert_eq!(
            compute_program_cache_key(&backend),
            compute_program_cache_key_from_fingerprint(fingerprint)
        );
        assert_ne!(fingerprint, compute_program_fingerprint(&changed));
        assert_ne!(
            compute_program_cache_key(&backend),
            compute_program_cache_key(&changed)
        );
    }

    #[test]
    fn compile_program_cached_recovers_from_corrupt_disk_entry() {
        let backend = lower_text("cache-program-corrupt.aivi", "value total:Int = 21 + 21\n");

        with_temp_cache_dir(|cache_root| {
            let key = compute_program_cache_key(&backend);
            let path = program_cache_path_in(cache_root, key);
            fs::create_dir_all(
                path.parent()
                    .expect("cache file should have a parent directory"),
            )
            .expect("program cache parent should be created");
            fs::write(&path, b"corrupt-program-cache")
                .expect("corrupt program cache entry should be written");

            let compiled = compile_program_cached_in_dir(cache_root, &backend)
                .expect("corrupt program cache should recompile");
            let loaded = load_cached_program_from(cache_root, key)
                .expect("recompiled program cache entry should deserialize cleanly");

            assert_eq!(compiled, loaded);
            assert_ne!(
                fs::read(&path).expect("recompiled cache file should be readable"),
                b"corrupt-program-cache"
            );
        });
    }

    #[test]
    fn compile_kernel_cached_recovers_from_corrupt_disk_entry() {
        let backend = lower_text("cache-kernel-corrupt.aivi", "value total:Int = 21 + 21\n");
        let total = backend.items()[find_item(&backend, "total")]
            .body
            .expect("total should lower into a body kernel");

        with_temp_cache_dir(|cache_root| {
            let fingerprint = compute_kernel_fingerprint(&backend, total);
            let path = kernel_cache_path_in(
                cache_root,
                compute_program_scoped_kernel_cache_key(&backend, fingerprint),
            );
            fs::create_dir_all(
                path.parent()
                    .expect("cache file should have a parent directory"),
            )
            .expect("kernel cache parent should be created");
            fs::write(&path, b"corrupt-kernel-cache")
                .expect("corrupt kernel cache entry should be written");

            let compiled = compile_kernel_cached_in_dir(cache_root, &backend, total)
                .expect("corrupt kernel cache should recompile");
            let loaded = load_cached_kernel_artifact_from(cache_root, &backend, fingerprint)
                .expect("recompiled kernel cache entry should deserialize cleanly");

            assert_eq!(compiled, loaded);
            assert_ne!(
                fs::read(&path).expect("recompiled cache file should be readable"),
                b"corrupt-kernel-cache"
            );
        });
    }

    #[test]
    fn cached_jit_kernel_scopes_artifacts_by_program_identity() {
        let first = lower_text(
            "cache-jit-program-scope.aivi",
            "value config:Int = 1\nvalue useConfig:Int = config\n",
        );
        let second = lower_text(
            "cache-jit-program-scope.aivi",
            "value config:Int = 2\nvalue useConfig:Int = config\n",
        );
        let first_kernel = first.items()[find_item(&first, "useConfig")]
            .body
            .expect("useConfig should lower into a body kernel");
        let second_kernel = second.items()[find_item(&second, "useConfig")]
            .body
            .expect("useConfig should lower into a body kernel");
        let first_fingerprint = compute_kernel_fingerprint(&first, first_kernel);
        let second_fingerprint = compute_kernel_fingerprint(&second, second_kernel);

        assert_eq!(
            first_fingerprint, second_fingerprint,
            "kernel fingerprint currently ignores dependent item body changes"
        );
        assert_ne!(
            compute_program_scoped_kernel_cache_key(&first, first_fingerprint),
            compute_program_scoped_kernel_cache_key(&second, second_fingerprint)
        );

        with_temp_cache_dir(|cache_root| {
            let compiled_first = compile_kernel_jit_cached_in_dir(cache_root, &first, first_kernel)
                .expect("first helper-backed kernel should compile");
            assert_eq!(
                call_i64_value(&compiled_first.caller, compiled_first.function, &[]),
                AbiValue::I64(1)
            );

            let compiled_second =
                compile_kernel_jit_cached_in_dir(cache_root, &second, second_kernel)
                    .expect("changed helper-backed kernel should miss the stale cache entry");
            assert_eq!(
                call_i64_value(&compiled_second.caller, compiled_second.function, &[]),
                AbiValue::I64(2)
            );
        });
    }

    #[test]
    fn cached_jit_kernel_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text("cache-jit-roundtrip.aivi", "value total:Int = 21 + 21\n");
        let total = backend.items()[find_item(&backend, "total")]
            .body
            .expect("total should lower into a body kernel");

        with_temp_cache_dir(|cache_root| {
            let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, total)
                .expect("JIT kernel should compile and store a replayable cache artifact");
            let fingerprint = compute_kernel_fingerprint(&backend, total);
            let artifact = load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                .expect("compiled JIT kernel should write a disk artifact");
            let replayed = instantiate_cached_jit_kernel(&backend, total, &artifact)
                .expect("serialized JIT artifact should replay into a live kernel");

            assert_eq!(
                compiled
                    .caller
                    .call(compiled.function, &[])
                    .expect("compiled kernel should execute"),
                AbiValue::I64(42)
            );
            assert_eq!(
                replayed
                    .caller
                    .call(replayed.function, &[])
                    .expect("replayed kernel should execute"),
                AbiValue::I64(42)
            );
        });
    }

    #[test]
    fn cached_jit_helper_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text(
            "cache-jit-helper-roundtrip.aivi",
            r#"
use aivi.core.bytes (
    append,
    repeat,
    slice
)

fun makeBlob:Bytes = seed:Int=>
    append (repeat seed 1) (slice 1 3 (repeat (seed + 1) 4))
"#,
        );
        let make_blob = backend.items()[find_item(&backend, "makeBlob")]
            .body
            .expect("makeBlob should lower into a body kernel");

        with_temp_cache_dir(|cache_root| {
            let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, make_blob).expect(
                "helper-backed JIT kernel should compile and persist a replayable cache artifact",
            );
            let fingerprint = compute_kernel_fingerprint(&backend, make_blob);
            let artifact = load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                .expect("compiled helper-backed JIT kernel should write a disk artifact");
            let replayed = instantiate_cached_jit_kernel(&backend, make_blob, &artifact)
                .expect("serialized helper-backed JIT artifact should replay into a live kernel");

            assert_eq!(
                artifact
                    .external_funcs
                    .iter()
                    .map(|symbol| symbol.as_ref())
                    .collect::<Vec<_>>(),
                vec!["aivi_bytes_append", "aivi_bytes_repeat", "aivi_bytes_slice"]
            );
            assert_eq!(
                call_pointer_bytes(&compiled.caller, compiled.function, &[AbiValue::I64(65)]),
                b"ABB".to_vec().into_boxed_slice()
            );
            assert_eq!(
                call_pointer_bytes(&replayed.caller, replayed.function, &[AbiValue::I64(65)]),
                b"ABB".to_vec().into_boxed_slice()
            );
        });
    }

    #[test]
    fn cached_jit_collection_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text(
            "cache-jit-collections-roundtrip.aivi",
            r#"
value ids:List Int = [1, 2, 3]

value tags:Set Text =
    Set [
        "news",
        "featured"
    ]

value headers:Map Text Text =
    Map {
        "Authorization": "Bearer demo",
        "Accept": "application/json"
    }
"#,
        );

        with_temp_cache_dir(|cache_root| {
            for (name, expected_funcs) in [
                ("ids", vec!["aivi_list_new"]),
                ("tags", vec!["aivi_set_new"]),
                ("headers", vec!["aivi_map_new"]),
            ] {
                let kernel = backend.items()[find_item(&backend, name)]
                    .body
                    .expect("collection item should lower into a body kernel");
                let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, kernel)
                    .expect(
                        "collection kernel should compile and persist a replayable cache artifact",
                    );
                let fingerprint = compute_kernel_fingerprint(&backend, kernel);
                let artifact =
                    load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                        .expect("compiled collection kernel should write a disk artifact");
                let replayed = instantiate_cached_jit_kernel(&backend, kernel, &artifact)
                    .expect("serialized collection artifact should replay into a live kernel");

                assert_eq!(
                    artifact
                        .external_funcs
                        .iter()
                        .map(|symbol| symbol.as_ref())
                        .collect::<Vec<_>>(),
                    expected_funcs
                );
                match name {
                    "ids" => {
                        assert_eq!(
                            call_i64_sequence(&compiled.caller, compiled.function, &[]),
                            vec![1, 2, 3]
                        );
                        assert_eq!(
                            call_i64_sequence(&replayed.caller, replayed.function, &[]),
                            vec![1, 2, 3]
                        );
                    }
                    "tags" => {
                        assert_eq!(
                            call_text_sequence(&compiled.caller, compiled.function, &[]),
                            vec!["news".to_owned(), "featured".to_owned()]
                        );
                        assert_eq!(
                            call_text_sequence(&replayed.caller, replayed.function, &[]),
                            vec!["news".to_owned(), "featured".to_owned()]
                        );
                    }
                    "headers" => {
                        assert_eq!(
                            call_text_map(&compiled.caller, compiled.function, &[]),
                            vec![
                                ("Authorization".to_owned(), "Bearer demo".to_owned()),
                                ("Accept".to_owned(), "application/json".to_owned()),
                            ]
                        );
                        assert_eq!(
                            call_text_map(&replayed.caller, replayed.function, &[]),
                            vec![
                                ("Authorization".to_owned(), "Bearer demo".to_owned()),
                                ("Accept".to_owned(), "application/json".to_owned()),
                            ]
                        );
                    }
                    _ => unreachable!(),
                }
            }
        });
    }

    #[test]
    fn cached_jit_imported_generic_matrix_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text(
            "cache-jit-matrix-roundtrip.aivi",
            r#"
use aivi.matrix (
    fromRows,
    width
)

value matrixWidth:Int =
    fromRows [
        [1, 2],
        [3, 4]
    ]
    ||> Ok matrix -> width matrix
    ||> Err _ -> 0
"#,
        );

        with_temp_cache_dir(|cache_root| {
            let kernel = backend.items()[find_item(&backend, "matrixWidth")]
                .body
                .expect("matrix width item should lower into a body kernel");
            let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, kernel)
                .expect("Matrix kernel should compile and persist a replayable cache artifact");
            let fingerprint = compute_kernel_fingerprint(&backend, kernel);
            let artifact = load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                .expect("compiled Matrix kernel should write a disk artifact");
            let replayed = instantiate_cached_jit_kernel(&backend, kernel, &artifact)
                .expect("serialized Matrix artifact should replay into a live kernel");

            assert!(
                artifact
                    .external_funcs
                    .iter()
                    .any(|symbol| symbol.as_ref() == "aivi_arena_alloc"),
                "Matrix artifact should retain arena allocation helper linkage"
            );
            assert_eq!(
                call_i64_value(&compiled.caller, compiled.function, &[]),
                AbiValue::I64(2)
            );
            assert_eq!(
                call_i64_value(&replayed.caller, replayed.function, &[]),
                AbiValue::I64(2)
            );
        });
    }

    #[test]
    fn cached_jit_numeric_helper_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text(
            "cache-jit-numeric-roundtrip.aivi",
            r#"
value decimalTotal:Decimal = 19.25d + 0.75d
value bigintTotal:BigInt = 123456789012345678901234567890n + 10n
"#,
        );

        with_temp_cache_dir(|cache_root| {
            for (name, expected_funcs) in [
                ("decimalTotal", vec!["aivi_decimal_add"]),
                ("bigintTotal", vec!["aivi_bigint_add"]),
            ] {
                let kernel = backend.items()[find_item(&backend, name)]
                    .body
                    .expect("numeric item should lower into a body kernel");
                let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, kernel)
                    .expect("numeric helper kernel should compile and persist a replayable cache artifact");
                let fingerprint = compute_kernel_fingerprint(&backend, kernel);
                let artifact =
                    load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                        .expect("compiled numeric helper kernel should write a disk artifact");
                let replayed = instantiate_cached_jit_kernel(&backend, kernel, &artifact)
                    .expect("serialized numeric helper artifact should replay into a live kernel");

                assert_eq!(
                    artifact
                        .external_funcs
                        .iter()
                        .map(|symbol| symbol.as_ref())
                        .collect::<Vec<_>>(),
                    expected_funcs
                );
                match name {
                    "decimalTotal" => {
                        let expected =
                            RuntimeDecimal::parse_literal("20.00d").expect("decimal should parse");
                        assert_eq!(
                            call_decimal_value(&compiled.caller, compiled.function, &[]),
                            expected
                        );
                        assert_eq!(
                            call_decimal_value(&replayed.caller, replayed.function, &[]),
                            expected
                        );
                    }
                    "bigintTotal" => {
                        let expected =
                            RuntimeBigInt::parse_literal("123456789012345678901234567900n")
                                .expect("bigint should parse");
                        assert_eq!(
                            call_bigint_value(&compiled.caller, compiled.function, &[]),
                            expected
                        );
                        assert_eq!(
                            call_bigint_value(&replayed.caller, replayed.function, &[]),
                            expected
                        );
                    }
                    _ => unreachable!(),
                }
            }
        });
    }

    #[test]
    fn cached_jit_inline_scalar_option_artifact_replays_after_disk_roundtrip() {
        let backend = lower_text(
            "cache-jit-inline-option-roundtrip.aivi",
            r#"
fun passMaybeInt:(Option Int) = value:(Option Int)=>    value
fun passMaybeFloat:(Option Float) = value:(Option Float)=>    value
fun passMaybeBool:(Option Bool) = value:(Option Bool)=>    value
"#,
        );

        with_temp_cache_dir(|cache_root| {
            for (name, argument, expected) in [
                (
                    "passMaybeInt",
                    AbiValue::I128(encode_inline_option_bits((-7i64) as u64)),
                    AbiValue::I128(encode_inline_option_bits((-7i64) as u64)),
                ),
                ("passMaybeInt", AbiValue::I128(0), AbiValue::I128(0)),
                (
                    "passMaybeFloat",
                    AbiValue::I128(encode_inline_option_bits(3.5f64.to_bits())),
                    AbiValue::I128(encode_inline_option_bits(3.5f64.to_bits())),
                ),
                (
                    "passMaybeBool",
                    AbiValue::I128(encode_inline_option_bits(0)),
                    AbiValue::I128(encode_inline_option_bits(0)),
                ),
                ("passMaybeBool", AbiValue::I128(0), AbiValue::I128(0)),
            ] {
                let kernel = backend.items()[find_item(&backend, name)]
                    .body
                    .expect("inline option function should lower into a body kernel");
                let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, kernel)
                    .expect("inline scalar option kernel should compile and persist a replayable cache artifact");
                let fingerprint = compute_kernel_fingerprint(&backend, kernel);
                let artifact =
                    load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint).expect(
                        "compiled inline scalar option kernel should write a disk artifact",
                    );
                let replayed = instantiate_cached_jit_kernel(&backend, kernel, &artifact).expect(
                    "serialized inline scalar option artifact should replay into a live kernel",
                );

                assert_eq!(
                    compiled
                        .caller
                        .call(compiled.function, &[argument])
                        .expect("compiled inline scalar option kernel should execute"),
                    expected
                );
                assert_eq!(
                    replayed
                        .caller
                        .call(replayed.function, &[argument])
                        .expect("replayed inline scalar option kernel should execute"),
                    expected
                );
            }
        });
    }

    #[test]
    fn compile_kernel_jit_cached_recovers_from_corrupt_disk_entry() {
        let backend = lower_text("cache-jit-corrupt.aivi", "value total:Int = 21 + 21\n");
        let total = backend.items()[find_item(&backend, "total")]
            .body
            .expect("total should lower into a body kernel");

        with_temp_cache_dir(|cache_root| {
            let fingerprint = compute_kernel_fingerprint(&backend, total);
            let path = jit_kernel_cache_path_in(
                cache_root,
                compute_program_scoped_kernel_cache_key(&backend, fingerprint),
            );
            fs::create_dir_all(
                path.parent()
                    .expect("JIT cache file should have a parent directory"),
            )
            .expect("JIT kernel cache parent should be created");
            fs::write(&path, b"corrupt-jit-kernel-cache")
                .expect("corrupt JIT kernel cache entry should be written");

            let compiled = compile_kernel_jit_cached_in_dir(cache_root, &backend, total)
                .expect("corrupt JIT kernel cache should recompile");
            let artifact = load_cached_jit_kernel_artifact_from(cache_root, &backend, fingerprint)
                .expect("recompiled JIT cache entry should deserialize cleanly");
            let replayed = instantiate_cached_jit_kernel(&backend, total, &artifact)
                .expect("recompiled JIT artifact should replay cleanly");

            assert_eq!(
                compiled
                    .caller
                    .call(compiled.function, &[])
                    .expect("recompiled kernel should execute"),
                AbiValue::I64(42)
            );
            assert_eq!(
                replayed
                    .caller
                    .call(replayed.function, &[])
                    .expect("replayed kernel should execute"),
                AbiValue::I64(42)
            );
            assert_ne!(
                fs::read(&path).expect("recompiled JIT cache file should be readable"),
                b"corrupt-jit-kernel-cache"
            );
        });
    }

    fn decode_pointer_bytes(value: AbiValue) -> Box<[u8]> {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from helper-backed bytes kernel, found {value:?}");
        };
        decode_len_prefixed_bytes(pointer)
            .expect("helper-backed bytes kernel should return len-prefixed bytes")
    }

    fn call_pointer_bytes(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> Box<[u8]> {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("helper-backed kernel should execute inside an active arena");
        decode_pointer_bytes(value)
    }

    fn call_i64_value(caller: &FunctionCaller, function: *const u8, args: &[AbiValue]) -> AbiValue {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("scalar kernel should execute inside an active arena")
    }

    fn call_i64_sequence(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> Vec<i64> {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("collection kernel should execute inside an active arena");
        decode_i64_sequence(value)
    }

    fn call_text_sequence(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> Vec<String> {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("collection kernel should execute inside an active arena");
        decode_text_sequence(value)
    }

    fn call_text_map(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> Vec<(String, String)> {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("map kernel should execute inside an active arena");
        decode_text_map(value)
    }

    fn call_decimal_value(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> RuntimeDecimal {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("decimal kernel should execute inside an active arena");
        decode_decimal_value(value)
    }

    fn call_bigint_value(
        caller: &FunctionCaller,
        function: *const u8,
        args: &[AbiValue],
    ) -> RuntimeBigInt {
        let arena = Rc::new(RefCell::new(AllocationArena::new()));
        let value = with_active_arena(Rc::clone(&arena), || caller.call(function, args))
            .expect("bigint kernel should execute inside an active arena");
        decode_bigint_value(value)
    }

    fn decode_i64_sequence(value: AbiValue) -> Vec<i64> {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from list kernel, found {value:?}");
        };
        let decoded =
            decode_marshaled_sequence(pointer).expect("list kernel should return a sequence");
        assert_eq!(decoded.element_size, 8);
        decoded
            .bytes
            .chunks_exact(decoded.element_size)
            .map(|chunk| i64::from_ne_bytes(chunk.try_into().expect("int cell should be 8 bytes")))
            .collect()
    }

    fn decode_text_sequence(value: AbiValue) -> Vec<String> {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from set kernel, found {value:?}");
        };
        let decoded =
            decode_marshaled_sequence(pointer).expect("set kernel should return a sequence");
        assert_eq!(decoded.element_size, std::mem::size_of::<usize>());
        decoded
            .bytes
            .chunks_exact(decoded.element_size)
            .map(|chunk| {
                let raw: [u8; std::mem::size_of::<usize>()] =
                    chunk.try_into().expect("text cell should store a pointer");
                let pointer = usize::from_ne_bytes(raw) as *const std::ffi::c_void;
                let bytes = decode_len_prefixed_bytes(pointer)
                    .expect("text cell pointer should decode to len-prefixed bytes");
                String::from_utf8(bytes.into_vec()).expect("text cell bytes should be valid UTF-8")
            })
            .collect()
    }

    fn decode_text_map(value: AbiValue) -> Vec<(String, String)> {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from map kernel, found {value:?}");
        };
        let decoded = decode_marshaled_map(pointer).expect("map kernel should return a map blob");
        let cell_size = std::mem::size_of::<usize>();
        assert_eq!(decoded.key_size, cell_size);
        assert_eq!(decoded.value_size, cell_size);
        decoded
            .bytes
            .chunks_exact(decoded.key_size + decoded.value_size)
            .map(|chunk| {
                let key_raw: [u8; std::mem::size_of::<usize>()] = chunk[..decoded.key_size]
                    .try_into()
                    .expect("map key cell should store a pointer");
                let value_raw: [u8; std::mem::size_of::<usize>()] = chunk
                    [decoded.key_size..decoded.key_size + decoded.value_size]
                    .try_into()
                    .expect("map value cell should store a pointer");
                let key_pointer = usize::from_ne_bytes(key_raw) as *const std::ffi::c_void;
                let value_pointer = usize::from_ne_bytes(value_raw) as *const std::ffi::c_void;
                let key = String::from_utf8(
                    decode_len_prefixed_bytes(key_pointer)
                        .expect("map key pointer should decode to len-prefixed bytes")
                        .into_vec(),
                )
                .expect("map key bytes should be valid UTF-8");
                let value = String::from_utf8(
                    decode_len_prefixed_bytes(value_pointer)
                        .expect("map value pointer should decode to len-prefixed bytes")
                        .into_vec(),
                )
                .expect("map value bytes should be valid UTF-8");
                (key, value)
            })
            .collect()
    }

    fn decode_decimal_value(value: AbiValue) -> RuntimeDecimal {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from decimal kernel, found {value:?}");
        };
        RuntimeDecimal::from_constant_bytes(
            read_decimal_constant_bytes(pointer)
                .expect("decimal kernel should return decimal bytes")
                .as_ref(),
        )
        .expect("decimal bytes should decode")
    }

    fn decode_bigint_value(value: AbiValue) -> RuntimeBigInt {
        let AbiValue::Pointer(pointer) = value else {
            panic!("expected pointer ABI value from bigint kernel, found {value:?}");
        };
        RuntimeBigInt::from_constant_bytes(
            read_bigint_constant_bytes(pointer)
                .expect("bigint kernel should return bigint bytes")
                .as_ref(),
        )
        .expect("bigint bytes should decode")
    }

    const fn encode_inline_option_bits(payload: u64) -> u128 {
        ((payload as u128) << 64) | 1
    }
}
