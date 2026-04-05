//! Persistent disk cache for compiled programs.
//!
//! Key: FNV hash of the backend Program's canonical debug representation
//!      XOR'd with a rotated hash of the compiler version string.
//! Format: custom binary with magic header for validation.

use std::{
    env,
    fs,
    hash::{Hash, Hasher},
    io::{Cursor, Read},
    path::PathBuf,
};

use rustc_hash::FxHasher;

use crate::{
    CompiledKernel, CompiledProgram, KernelId,
    codegen::{CodegenErrors, compile_program},
    program::Program,
};

/// Magic bytes: ASCII "AIVI" + format version byte.
const CACHE_MAGIC: &[u8; 5] = b"AIVI\x01";

const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Compute a stable 64-bit cache key combining the backend program content
/// with the compiler version so stale entries are automatically invalidated
/// after an upgrade.
pub fn compute_program_cache_key(program: &Program) -> u64 {
    let mut hasher = FxHasher::default();
    format!("{program:?}").hash(&mut hasher);
    let program_hash = hasher.finish();

    let mut version_hasher = FxHasher::default();
    COMPILER_VERSION.hash(&mut version_hasher);
    let version_hash = version_hasher.finish();

    program_hash ^ version_hash.rotate_left(32)
}

fn cache_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("aivi").join("compiled"))
}

fn cache_path(key: u64) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("{key:016x}.bin")))
}

fn serialize(compiled: &CompiledProgram) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(CACHE_MAGIC);

    let obj = compiled.object();
    buf.extend_from_slice(&(obj.len() as u64).to_le_bytes());
    buf.extend_from_slice(obj);

    let kernels = compiled.kernels();
    buf.extend_from_slice(&(kernels.len() as u32).to_le_bytes());
    for kernel in kernels {
        buf.extend_from_slice(&kernel.kernel.as_raw().to_le_bytes());

        let sym = kernel.symbol.as_bytes();
        buf.extend_from_slice(&(sym.len() as u32).to_le_bytes());
        buf.extend_from_slice(sym);

        let clif = kernel.clif.as_bytes();
        buf.extend_from_slice(&(clif.len() as u32).to_le_bytes());
        buf.extend_from_slice(clif);

        buf.extend_from_slice(&(kernel.code_size as u64).to_le_bytes());
    }
    buf
}

fn deserialize(bytes: &[u8]) -> Option<CompiledProgram> {
    let mut c = Cursor::new(bytes);

    fn read_u32(c: &mut Cursor<&[u8]>) -> Option<u32> {
        let mut buf = [0u8; 4];
        c.read_exact(&mut buf).ok()?;
        Some(u32::from_le_bytes(buf))
    }
    fn read_u64(c: &mut Cursor<&[u8]>) -> Option<u64> {
        let mut buf = [0u8; 8];
        c.read_exact(&mut buf).ok()?;
        Some(u64::from_le_bytes(buf))
    }
    fn read_boxed_str(c: &mut Cursor<&[u8]>) -> Option<Box<str>> {
        let len = read_u32(c)? as usize;
        let mut buf = vec![0u8; len];
        c.read_exact(&mut buf).ok()?;
        String::from_utf8(buf).ok().map(String::into_boxed_str)
    }

    let mut magic = [0u8; 5];
    c.read_exact(&mut magic).ok()?;
    if &magic != CACHE_MAGIC {
        return None;
    }

    let obj_len = read_u64(&mut c)? as usize;
    let mut object = vec![0u8; obj_len];
    c.read_exact(&mut object).ok()?;

    let kernel_count = read_u32(&mut c)? as usize;
    let mut kernels = Vec::with_capacity(kernel_count);
    for _ in 0..kernel_count {
        let kernel_raw = read_u32(&mut c)?;
        let symbol = read_boxed_str(&mut c)?;
        let clif = read_boxed_str(&mut c)?;
        let code_size = read_u64(&mut c)? as usize;
        kernels.push(CompiledKernel {
            kernel: KernelId::from_raw(kernel_raw),
            symbol,
            clif,
            code_size,
        });
    }

    Some(CompiledProgram::new(object, kernels))
}

/// Load a cached `CompiledProgram` for the given key, if a valid entry exists.
pub fn load_cached_program(key: u64) -> Option<CompiledProgram> {
    let path = cache_path(key)?;
    let bytes = fs::read(&path).ok()?;
    deserialize(&bytes)
}

/// Persist a `CompiledProgram` to the disk cache under the given key.
/// Silently ignores I/O failures so a missing or read-only cache never breaks compilation.
pub fn store_cached_program(key: u64, compiled: &CompiledProgram) {
    let Some(path) = cache_path(key) else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serialize(compiled));
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
