use std::{cell::RefCell, ffi::c_void, ptr, rc::Rc, slice};

use libffi::middle::{Arg, Cif, CodePtr, Type};

thread_local! {
    static ACTIVE_ARENA: RefCell<Option<Rc<RefCell<AllocationArena>>>> = const { RefCell::new(None) };
}

#[derive(Debug, Default)]
pub struct AllocationArena {
    allocations: Vec<Box<[u8]>>,
}

impl AllocationArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store_len_prefixed_bytes(&mut self, bytes: &[u8]) -> *const c_void {
        let mut encoded = Vec::with_capacity(8 + bytes.len());
        encoded.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        encoded.extend_from_slice(bytes);
        let cell = encoded.into_boxed_slice();
        let pointer = cell.as_ptr();
        self.allocations.push(cell);
        pointer.cast()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbiValueKind {
    I8,
    I64,
    F64,
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AbiValue {
    I8(i8),
    I64(i64),
    F64(f64),
    Pointer(*const c_void),
}

impl AbiValue {
    pub const fn kind(self) -> AbiValueKind {
        match self {
            Self::I8(_) => AbiValueKind::I8,
            Self::I64(_) => AbiValueKind::I64,
            Self::F64(_) => AbiValueKind::F64,
            Self::Pointer(_) => AbiValueKind::Pointer,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSignature {
    args: Box<[AbiValueKind]>,
    result: AbiValueKind,
}

impl CallSignature {
    pub fn new(args: impl Into<Box<[AbiValueKind]>>, result: AbiValueKind) -> Self {
        Self {
            args: args.into(),
            result,
        }
    }

    pub fn args(&self) -> &[AbiValueKind] {
        &self.args
    }

    pub const fn result(&self) -> AbiValueKind {
        self.result
    }
}

#[derive(Debug)]
pub struct FunctionCaller {
    signature: CallSignature,
    cif: Cif,
}

impl FunctionCaller {
    pub fn new(signature: CallSignature) -> Self {
        let arg_types = signature.args.iter().copied().map(type_for_abi_kind);
        let cif = Cif::new(arg_types, type_for_abi_kind(signature.result));
        Self { signature, cif }
    }

    pub fn signature(&self) -> &CallSignature {
        &self.signature
    }

    pub fn call(&self, function: *const u8, args: &[AbiValue]) -> Result<AbiValue, CallError> {
        if args.len() != self.signature.args.len() {
            return Err(CallError::ArityMismatch {
                expected: self.signature.args.len(),
                found: args.len(),
            });
        }

        let mut owned_args = Vec::with_capacity(args.len());
        for (index, (value, kind)) in args
            .iter()
            .copied()
            .zip(self.signature.args.iter().copied())
            .enumerate()
        {
            owned_args.push(OwnedArg::new(value, kind).map_err(|found| {
                CallError::ArgumentTypeMismatch {
                    index,
                    expected: kind,
                    found,
                }
            })?);
        }

        let ffi_args: Vec<_> = owned_args.iter().map(OwnedArg::as_arg).collect();
        let code_ptr = CodePtr(function as *mut c_void);
        let result = match self.signature.result {
            AbiValueKind::I8 => {
                // SAFETY: `FunctionCaller` only constructs a CIF from the stored signature,
                // and `OwnedArg::new` ensures the runtime arguments match that signature.
                AbiValue::I8(unsafe { self.cif.call::<i8>(code_ptr, &ffi_args) })
            }
            AbiValueKind::I64 => {
                // SAFETY: same reasoning as above.
                AbiValue::I64(unsafe { self.cif.call::<i64>(code_ptr, &ffi_args) })
            }
            AbiValueKind::F64 => {
                // SAFETY: same reasoning as above.
                AbiValue::F64(unsafe { self.cif.call::<f64>(code_ptr, &ffi_args) })
            }
            AbiValueKind::Pointer => {
                // SAFETY: same reasoning as above.
                AbiValue::Pointer(unsafe { self.cif.call::<*const c_void>(code_ptr, &ffi_args) })
            }
        };
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    ArityMismatch {
        expected: usize,
        found: usize,
    },
    ArgumentTypeMismatch {
        index: usize,
        expected: AbiValueKind,
        found: AbiValueKind,
    },
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArityMismatch { expected, found } => write!(
                f,
                "foreign call received {found} argument(s), expected {expected}"
            ),
            Self::ArgumentTypeMismatch {
                index,
                expected,
                found,
            } => write!(
                f,
                "foreign call argument {} had ABI kind {found:?}, expected {expected:?}",
                index + 1
            ),
        }
    }
}

impl std::error::Error for CallError {}

pub fn with_active_arena<R>(arena: Rc<RefCell<AllocationArena>>, f: impl FnOnce() -> R) -> R {
    ACTIVE_ARENA.with(|slot| {
        let previous = slot.replace(Some(arena));
        let result = f();
        slot.replace(previous);
        result
    })
}

pub fn encode_len_prefixed_bytes(bytes: &[u8], arena: &mut AllocationArena) -> *const c_void {
    arena.store_len_prefixed_bytes(bytes)
}

pub fn decode_len_prefixed_bytes(pointer: *const c_void) -> Option<Box<[u8]>> {
    // SAFETY: callers only hand us pointers produced by the AIVI backend's byte-sequence
    // contract (u64 little-endian length prefix followed by that many bytes) or null.
    unsafe {
        let bytes = read_len_prefixed_bytes(pointer.cast())?;
        Some(bytes.into())
    }
}

pub fn lookup_runtime_symbol(symbol: &str) -> Option<*const u8> {
    match symbol {
        "aivi_text_concat" => Some(aivi_text_concat as *const () as *const u8),
        _ => None,
    }
}

fn type_for_abi_kind(kind: AbiValueKind) -> Type {
    match kind {
        AbiValueKind::I8 => Type::i8(),
        AbiValueKind::I64 => Type::i64(),
        AbiValueKind::F64 => Type::f64(),
        AbiValueKind::Pointer => Type::pointer(),
    }
}

#[derive(Clone, Copy, Debug)]
enum OwnedArg {
    I8(i8),
    I64(i64),
    F64(f64),
    Pointer(*const c_void),
}

impl OwnedArg {
    fn new(value: AbiValue, expected: AbiValueKind) -> Result<Self, AbiValueKind> {
        match (value, expected) {
            (AbiValue::I8(value), AbiValueKind::I8) => Ok(Self::I8(value)),
            (AbiValue::I64(value), AbiValueKind::I64) => Ok(Self::I64(value)),
            (AbiValue::F64(value), AbiValueKind::F64) => Ok(Self::F64(value)),
            (AbiValue::Pointer(value), AbiValueKind::Pointer) => Ok(Self::Pointer(value)),
            (value, _) => Err(value.kind()),
        }
    }

    fn as_arg(&self) -> Arg<'_> {
        match self {
            Self::I8(value) => Arg::new(value),
            Self::I64(value) => Arg::new(value),
            Self::F64(value) => Arg::new(value),
            Self::Pointer(value) => Arg::new(value),
        }
    }
}

extern "C" fn aivi_text_concat(count: i64, segments: *const *const u8) -> *const u8 {
    with_current_arena(|arena| {
        if count < 0 || segments.is_null() {
            return ptr::null();
        }
        // SAFETY: the JIT helper ABI passes `count` contiguous segment pointers.
        let segment_ptrs = unsafe { slice::from_raw_parts(segments, count as usize) };
        let mut joined = Vec::new();
        for &segment in segment_ptrs {
            // SAFETY: each segment pointer follows the same len-prefixed byte contract.
            let Some(bytes) = (unsafe { read_len_prefixed_bytes(segment) }) else {
                return ptr::null();
            };
            joined.extend_from_slice(bytes);
        }
        arena.store_len_prefixed_bytes(&joined).cast()
    })
    .unwrap_or(ptr::null())
}

fn with_current_arena<R>(f: impl FnOnce(&mut AllocationArena) -> R) -> Option<R> {
    ACTIVE_ARENA.with(|slot| {
        let arena = slot.borrow().as_ref()?.clone();
        let mut arena = arena.borrow_mut();
        Some(f(&mut arena))
    })
}

unsafe fn read_len_prefixed_bytes<'a>(pointer: *const u8) -> Option<&'a [u8]> {
    if pointer.is_null() {
        return None;
    }
    let mut prefix = [0u8; 8];
    // SAFETY: caller guarantees `pointer` addresses at least the 8-byte length prefix.
    unsafe { ptr::copy_nonoverlapping(pointer, prefix.as_mut_ptr(), prefix.len()) };
    let len = u64::from_le_bytes(prefix) as usize;
    // SAFETY: caller guarantees the contract stores exactly `len` bytes immediately after prefix.
    Some(unsafe { slice::from_raw_parts(pointer.add(prefix.len()), len) })
}
