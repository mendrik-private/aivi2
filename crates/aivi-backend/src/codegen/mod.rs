use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};

use rayon::prelude::*;
use rustc_hash::FxHasher;

use aivi_ffi_call::{AbiValueKind, CallSignature, FunctionCaller};
use aivi_hir::IntrinsicValue;
use cranelift_codegen::{
    binemit::Reloc,
    control::ControlPlane,
    ir::{
        AbiParam, BlockArg, InstBuilder, MemFlags, Type, UserFuncName, Value,
        condcodes::{FloatCC, IntCC},
        immediates::Ieee64,
        types,
    },
    isa::OwnedTargetIsa,
    print_errors::pretty_verifier_error,
    settings::{self, Configurable},
    verify_function,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{
    DataDescription, DataId, FuncId, Linkage, Module, ModuleReloc, ModuleRelocTarget,
    default_libcall_names,
};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::{
    AbiPassMode, BinaryOperator, BuiltinTerm, CallingConventionKind, EnvSlotId, ItemId, Kernel,
    KernelExprId, KernelExprKind, KernelId, KernelOriginKind, Layout, LayoutId, LayoutKind,
    ParameterRole, PrimitiveType, Program, RuntimeMap, RuntimeMapEntry, RuntimeRecordField,
    RuntimeValue, SubjectRef, UnaryOperator, ValidationError, describe_expr_kind,
    numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat},
    program::ItemKind,
    validate_program,
};

include!("artifacts.rs");
include!("errors_api.rs");
include!("specialized.rs");
include!("compiler.rs");
include!("helpers.rs");
