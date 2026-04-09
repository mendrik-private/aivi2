//! Persistent cache surfaces for compiled backend artifacts.
//!
//! Program cache key: FNV hash of the backend program's canonical debug representation XOR'd with
//! a rotated hash of the compiler version string.
//! Kernel cache key: stable kernel fingerprint XOR'd with that same rotated compiler-version hash.
//! Artifact format: custom binary with magic headers for validation.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    env, fs,
    hash::{Hash, Hasher},
    io::{Cursor, Read},
    path::PathBuf,
};

use rustc_hash::FxHasher;

use crate::{
    CompiledKernel, CompiledKernelArtifact, CompiledProgram, KernelFingerprint, KernelId,
    codegen::{CodegenErrors, compile_kernel, compile_program, compute_kernel_fingerprint},
    program::Program,
};

/// Magic bytes: ASCII "AIVI" + format version byte.
const PROGRAM_CACHE_MAGIC: &[u8; 5] = b"AIVI\x02";
/// Magic bytes: ASCII "AIVK" + format version byte.
const KERNEL_CACHE_MAGIC: &[u8; 5] = b"AIVK\x01";

const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// In-memory cache for per-kernel object artifacts owned by the backend layer.
#[derive(Clone, Debug, Default)]
pub struct BackendKernelArtifactCache {
    artifacts: BTreeMap<KernelFingerprint, CompiledKernelArtifact>,
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

    pub fn get(&self, fingerprint: KernelFingerprint) -> Option<&CompiledKernelArtifact> {
        self.artifacts.get(&fingerprint)
    }

    pub fn get_by_kernel(
        &self,
        program: &Program,
        kernel_id: KernelId,
    ) -> Option<&CompiledKernelArtifact> {
        if !program.kernels().contains(kernel_id) {
            return None;
        }
        self.get(compute_kernel_fingerprint(program, kernel_id))
    }

    pub fn insert(&mut self, artifact: CompiledKernelArtifact) -> Option<CompiledKernelArtifact> {
        self.artifacts.insert(artifact.fingerprint(), artifact)
    }

    pub fn get_or_compile(
        &mut self,
        program: &Program,
        kernel_id: KernelId,
    ) -> Result<&CompiledKernelArtifact, CodegenErrors> {
        if !program.kernels().contains(kernel_id) {
            let error = compile_kernel(program, kernel_id)
                .err()
                .expect("compiling a missing kernel should produce a backend codegen error");
            return Err(error);
        }
        let fingerprint = compute_kernel_fingerprint(program, kernel_id);
        match self.artifacts.entry(fingerprint) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let artifact = compile_kernel(program, kernel_id)?;
                Ok(entry.insert(artifact))
            }
        }
    }
}

/// Compute a stable 64-bit cache key combining the backend program content
/// with the compiler version so stale entries are automatically invalidated
/// after an upgrade.
pub fn compute_program_cache_key(program: &Program) -> u64 {
    let mut hasher = FxHasher::default();
    format!("{program:?}").hash(&mut hasher);
    hasher.finish() ^ compiler_version_hash().rotate_left(32)
}

/// Compute a disk-cache key for one kernel artifact from its stable content fingerprint.
pub fn compute_kernel_cache_key(fingerprint: KernelFingerprint) -> u64 {
    fingerprint.as_raw() ^ compiler_version_hash().rotate_left(32)
}

fn compiler_version_hash() -> u64 {
    let mut version_hasher = FxHasher::default();
    COMPILER_VERSION.hash(&mut version_hasher);
    version_hasher.finish()
}

fn cache_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("aivi").join("compiled"))
}

fn program_cache_path(key: u64) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("program-{key:016x}.bin")))
}

fn kernel_cache_path(key: u64) -> Option<PathBuf> {
    Some(cache_dir()?.join("kernels").join(format!("{key:016x}.bin")))
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

fn read_boxed_str(cursor: &mut Cursor<&[u8]>) -> Option<Box<str>> {
    let len = read_u32(cursor)? as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok().map(String::into_boxed_str)
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

/// Load a cached `CompiledProgram` for the given key, if a valid entry exists.
pub fn load_cached_program(key: u64) -> Option<CompiledProgram> {
    let path = program_cache_path(key)?;
    let bytes = fs::read(&path).ok()?;
    deserialize_program(&bytes)
}

/// Persist a `CompiledProgram` to the disk cache under the given key.
/// Silently ignores I/O failures so a missing or read-only cache never breaks compilation.
pub fn store_cached_program(key: u64, compiled: &CompiledProgram) {
    let Some(path) = program_cache_path(key) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize_program(compiled));
}

/// Load a cached per-kernel object artifact, if a valid entry exists.
pub fn load_cached_kernel_artifact(
    fingerprint: KernelFingerprint,
) -> Option<CompiledKernelArtifact> {
    let path = kernel_cache_path(compute_kernel_cache_key(fingerprint))?;
    let bytes = fs::read(&path).ok()?;
    let artifact = deserialize_kernel_artifact(&bytes)?;
    (artifact.fingerprint() == fingerprint).then_some(artifact)
}

/// Persist a per-kernel object artifact to the disk cache.
/// Silently ignores I/O failures so a missing or read-only cache never breaks compilation.
pub fn store_cached_kernel_artifact(
    fingerprint: KernelFingerprint,
    artifact: &CompiledKernelArtifact,
) {
    if artifact.fingerprint() != fingerprint {
        return;
    }
    let Some(path) = kernel_cache_path(compute_kernel_cache_key(fingerprint)) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize_kernel_artifact(artifact));
}

/// Compile a backend program, consulting the disk cache first to skip Cranelift
/// codegen for unchanged programs. Falls back to full compilation on cache miss
/// or any deserialization error.
pub fn compile_program_cached(program: &Program) -> Result<CompiledProgram, CodegenErrors> {
    let key = compute_program_cache_key(program);
    if let Some(cached) = load_cached_program(key) {
        return Ok(cached);
    }
    let compiled = compile_program(program)?;
    store_cached_program(key, &compiled);
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
    let fingerprint = compute_kernel_fingerprint(program, kernel_id);
    if let Some(cached) = load_cached_kernel_artifact(fingerprint) {
        return Ok(cached);
    }
    let compiled = compile_kernel(program, kernel_id)?;
    store_cached_kernel_artifact(fingerprint, &compiled);
    Ok(compiled)
}
