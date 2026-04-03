use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

use aivi_hir::IntrinsicValue;
use cranelift_codegen::{
    ir::{
        AbiParam, BlockArg, InstBuilder, MemFlags, Type, UserFuncName, Value,
        condcodes::{FloatCC, IntCC},
        immediates::Ieee64,
        types,
    },
    print_errors::pretty_verifier_error,
    settings::{self, Configurable},
    verify_function,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::{
    AbiPassMode, BinaryOperator, BuiltinTerm, CallingConventionKind, EnvSlotId, ItemId, Kernel,
    KernelExprId, KernelExprKind, KernelId, KernelOriginKind, Layout, LayoutId, LayoutKind,
    ParameterRole, PrimitiveType, Program, RuntimeMap, RuntimeMapEntry, RuntimeRecordField,
    RuntimeValue, SubjectRef, UnaryOperator, ValidationError, describe_expr_kind,
    numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat},
    validate_program,
};

/// Cranelift compilation results for one backend program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledProgram {
    object: Vec<u8>,
    kernels: Vec<CompiledKernel>,
    kernel_index: BTreeMap<KernelId, usize>,
}

impl CompiledProgram {
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
}

/// Cranelift artifacts for one backend kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledKernel {
    pub kernel: KernelId,
    pub symbol: Box<str>,
    pub clif: Box<str>,
    pub code_size: usize,
}

pub type CodegenErrors = aivi_base::ErrorCollection<CodegenError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    InvalidBackendProgram(ValidationError),
    HostIsaUnavailable {
        message: Box<str>,
    },
    TargetIsaCreation {
        message: Box<str>,
    },
    ObjectModuleCreation {
        message: Box<str>,
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
    if let Err(errors) = validate_program(program) {
        return Err(CodegenErrors::new(
            errors
                .into_errors()
                .into_iter()
                .map(CodegenError::InvalidBackendProgram)
                .collect(),
        ));
    }

    let compiler = CraneliftCompiler::new(program).map_err(wrap_one)?;
    compiler.prevalidate()?;
    compiler.compile()
}

struct CraneliftCompiler<'a> {
    program: &'a Program,
    module: ObjectModule,
    declared_functions: BTreeMap<KernelId, FuncId>,
    declared_signal_slots: BTreeMap<ItemId, DataId>,
    declared_imported_item_slots: BTreeMap<ItemId, DataId>,
    declared_callable_descriptors: BTreeMap<ItemId, DataId>,
    declared_external_funcs: BTreeMap<Box<str>, FuncId>,
    function_builder_ctx: FunctionBuilderContext,
    next_data_symbol: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectApplyPlan {
    Item {
        body: KernelId,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinCallPlan {
    OptionSome(OptionCodegenContract),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntrinsicCallPlan {
    BytesLength,
    BytesGet,
    BytesFromText,
    BytesToText,
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
enum NativeCompareKind {
    Integer,
    Float,
    Decimal,
    BigInt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeArithmeticKind {
    Integer,
    Decimal,
    BigInt,
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

impl<'a> CraneliftCompiler<'a> {
    fn new(program: &'a Program) -> Result<Self, CodegenError> {
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
        let isa = isa_builder
            .finish(settings::Flags::new(flags))
            .map_err(|error| CodegenError::TargetIsaCreation {
                message: error.to_string().into_boxed_str(),
            })?;
        let module = ObjectModule::new(
            ObjectBuilder::new(isa, "aivi_backend", default_libcall_names()).map_err(|error| {
                CodegenError::ObjectModuleCreation {
                    message: error.to_string().into_boxed_str(),
                }
            })?,
        );

        Ok(Self {
            program,
            module,
            declared_functions: BTreeMap::new(),
            declared_signal_slots: BTreeMap::new(),
            declared_imported_item_slots: BTreeMap::new(),
            declared_callable_descriptors: BTreeMap::new(),
            declared_external_funcs: BTreeMap::new(),
            function_builder_ctx: FunctionBuilderContext::new(),
            next_data_symbol: 0,
        })
    }

    /// Ambient prelude kernels (e.g. `__aivi_list_*`) use runtime-only
    /// intrinsics (`Reduce`, `Append`) and are interpreted, never compiled.
    fn is_ambient_kernel(&self, kernel: &Kernel) -> bool {
        self.program.items()[kernel.origin.item]
            .name
            .starts_with("__aivi_")
    }

    fn prevalidate(&self) -> Result<(), CodegenErrors> {
        let mut errors = Vec::new();

        for (kernel_id, kernel) in self.program.kernels().iter() {
            if self.is_ambient_kernel(kernel) {
                continue;
            }
            match kernel.convention.kind {
                CallingConventionKind::RuntimeKernelV1 => {}
            }
            for parameter in &kernel.convention.parameters {
                if let Err(error) = self.materialize_signature_type(
                    kernel_id,
                    parameter.layout,
                    parameter.pass_mode,
                    &format!("parameter {}", parameter.role),
                ) {
                    errors.push(error);
                }
            }
            if let Err(error) = self.materialize_signature_type(
                kernel_id,
                kernel.convention.result.layout,
                kernel.convention.result.pass_mode,
                "result",
            ) {
                errors.push(error);
            }

            let mut work = vec![kernel.root];
            let mut visited = HashSet::new();
            while let Some(expr_id) = work.pop() {
                if !visited.insert(expr_id) {
                    continue;
                }

                let expr = &kernel.exprs()[expr_id];
                match self.can_materialize_static_expression(kernel_id, kernel, expr_id) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                }
                match &expr.kind {
                    KernelExprKind::Subject(SubjectRef::Input)
                    | KernelExprKind::Subject(SubjectRef::Inline(_))
                    | KernelExprKind::Environment(_) => {}
                    KernelExprKind::OptionSome { payload } => {
                        if let Err(error) = self.require_option_codegen_contract(
                            kernel_id,
                            kernel,
                            expr_id,
                            Some(*payload),
                            expr.layout,
                            "Some carrier",
                        ) {
                            errors.push(error);
                        }
                        work.push(*payload);
                    }
                    KernelExprKind::OptionNone => {
                        if let Err(error) = self.require_option_codegen_contract(
                            kernel_id,
                            kernel,
                            expr_id,
                            None,
                            expr.layout,
                            "None carrier",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Integer(_) => {
                        if let Err(error) = self.require_int_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "integer literal",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Float(_) => {
                        if let Err(error) = self.require_float_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "Float literal",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Decimal(_) => {
                        if let Err(error) = self.require_decimal_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "Decimal literal",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::BigInt(_) => {
                        if let Err(error) = self.require_bigint_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "BigInt literal",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Builtin(BuiltinTerm::True | BuiltinTerm::False) => {
                        if let Err(error) = self.require_bool_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "Bool literal",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Builtin(BuiltinTerm::None) => {
                        if let Err(error) = self.require_option_codegen_contract(
                            kernel_id,
                            kernel,
                            expr_id,
                            None,
                            expr.layout,
                            "None constructor",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Text(text) => {
                        if let Err(error) = self.require_text_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "Text literal",
                        ) {
                            errors.push(error);
                        }
                        // Allow both static and dynamic (interpolation) text
                        for segment in &text.segments {
                            if let crate::TextSegment::Interpolation {
                                expr: interp_expr, ..
                            } = segment
                            {
                                work.push(*interp_expr);
                            }
                        }
                    }
                    KernelExprKind::Tuple(elements) => {
                        for elem in elements {
                            work.push(*elem);
                        }
                    }
                    KernelExprKind::Record(fields) => {
                        for field in fields {
                            work.push(field.value);
                        }
                    }
                    KernelExprKind::Projection { base, .. } => {
                        let Some(base_layout) = (match base {
                            crate::ProjectionBase::Subject(SubjectRef::Input) => {
                                kernel.input_subject
                            }
                            crate::ProjectionBase::Subject(SubjectRef::Inline(slot)) => Some(
                                *kernel.inline_subjects.get(slot.index()).expect(
                                    "validated backend kernels keep inline subject layouts aligned with codegen",
                                ),
                            ),
                            crate::ProjectionBase::Expr(base_expr) => {
                                work.push(*base_expr);
                                Some(kernel.exprs()[*base_expr].layout)
                            }
                        }) else {
                            continue;
                        };

                        if let Err(error) =
                            self.resolve_projection_steps(kernel_id, kernel, expr_id, base_layout)
                        {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Pipe(pipe) => {
                        work.push(pipe.head);
                        let mut current_layout = kernel.exprs()[pipe.head].layout;
                        for (stage_index, stage) in pipe.stages.iter().enumerate() {
                            if let Err(error) = self.require_layout_match(
                                kernel_id,
                                expr_id,
                                stage.input_layout,
                                current_layout,
                                &format!("inline-pipe stage {stage_index} input"),
                            ) {
                                errors.push(error);
                            }
                            match &stage.kind {
                                crate::InlinePipeStageKind::Transform { expr, .. } => {
                                    work.push(*expr);
                                    if let Err(error) = self.require_layout_match(
                                        kernel_id,
                                        expr_id,
                                        stage.result_layout,
                                        kernel.exprs()[*expr].layout,
                                        &format!("inline-pipe stage {stage_index} result"),
                                    ) {
                                        errors.push(error);
                                    }
                                }
                                crate::InlinePipeStageKind::Tap { expr } => {
                                    work.push(*expr);
                                    if let Err(error) = self.require_layout_match(
                                        kernel_id,
                                        expr_id,
                                        stage.result_layout,
                                        stage.input_layout,
                                        &format!("inline-pipe tap stage {stage_index} result"),
                                    ) {
                                        errors.push(error);
                                    }
                                }
                                crate::InlinePipeStageKind::Debug { .. } => {
                                    // Debug is a no-op in compiled code (observability only).
                                }
                                crate::InlinePipeStageKind::Gate { .. } => {
                                    if let Err(error) = self.require_inline_pipe_gate_contract(
                                        kernel_id,
                                        expr_id,
                                        stage_index,
                                        stage.input_layout,
                                        stage.result_layout,
                                    ) {
                                        errors.push(error);
                                    }
                                }
                                crate::InlinePipeStageKind::Case { arms } => {
                                    for arm in arms {
                                        work.push(arm.body);
                                    }
                                }
                                crate::InlinePipeStageKind::TruthyFalsy { truthy, falsy } => {
                                    work.push(truthy.body);
                                    work.push(falsy.body);
                                }
                                crate::InlinePipeStageKind::FanOut { map_expr } => {
                                    work.push(*map_expr);
                                }
                            }
                            current_layout = stage.result_layout;
                        }
                        if let Err(error) = self.require_layout_match(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            current_layout,
                            "inline-pipe result",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Unary { expr: inner, .. } => {
                        work.push(*inner);
                        if let Err(error) = self.require_bool_expression(
                            kernel_id,
                            *inner,
                            kernel.exprs()[*inner].layout,
                            "logical not operand",
                        ) {
                            errors.push(error);
                        }
                        if let Err(error) = self.require_bool_expression(
                            kernel_id,
                            expr_id,
                            expr.layout,
                            "logical not result",
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Binary {
                        left,
                        operator,
                        right,
                    } => {
                        work.push(*right);
                        work.push(*left);
                        match operator {
                            BinaryOperator::Add
                            | BinaryOperator::Subtract
                            | BinaryOperator::Multiply
                            | BinaryOperator::Divide
                            | BinaryOperator::Modulo => {
                                if let Err(error) = self.require_arithmetic_expression_triple(
                                    kernel_id, kernel, expr_id, *left, *right,
                                ) {
                                    errors.push(error);
                                }
                            }
                            BinaryOperator::GreaterThan
                            | BinaryOperator::LessThan
                            | BinaryOperator::GreaterThanOrEqual
                            | BinaryOperator::LessThanOrEqual => {
                                if let Err(error) = self.require_ordered_expression_pair(
                                    kernel_id, kernel, expr_id, *left, *right,
                                ) {
                                    errors.push(error);
                                }
                            }
                            BinaryOperator::Equals | BinaryOperator::NotEquals => {
                                if let Err(error) = self.require_equatable_expression_pair(
                                    kernel_id, kernel, expr_id, *left, *right,
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_bool_expression(
                                    kernel_id,
                                    expr_id,
                                    expr.layout,
                                    "equality result",
                                ) {
                                    errors.push(error);
                                }
                            }
                            BinaryOperator::And | BinaryOperator::Or => {
                                if let Err(error) = self.require_bool_expression(
                                    kernel_id,
                                    *left,
                                    kernel.exprs()[*left].layout,
                                    "logical lhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_bool_expression(
                                    kernel_id,
                                    *right,
                                    kernel.exprs()[*right].layout,
                                    "logical rhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_bool_expression(
                                    kernel_id,
                                    expr_id,
                                    expr.layout,
                                    "logical result",
                                ) {
                                    errors.push(error);
                                }
                            }
                        }
                    }
                    KernelExprKind::Item(item) => {
                        if let Err(error) = self.plan_item_reference(kernel_id, expr_id, *item) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::IntrinsicValue(intrinsic) => {
                        if let Err(error) = self.require_compilable_intrinsic_value(
                            kernel_id,
                            expr_id,
                            *intrinsic,
                            expr.layout,
                        ) {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::Apply { callee, arguments } => {
                        for argument in arguments {
                            work.push(*argument);
                        }
                        if let Err(error) =
                            self.resolve_direct_apply_plan(kernel_id, expr_id, *callee, arguments)
                        {
                            errors.push(error);
                        }
                    }
                    KernelExprKind::SumConstructor(_) => {
                        // Zero-field sum constructors are handled as static sum singletons.
                        // Multi-field constructors are only supported as callees in Apply.
                    }
                    KernelExprKind::List(elements) => {
                        for elem in elements {
                            work.push(*elem);
                        }
                    }
                    KernelExprKind::Set(elements) => {
                        for elem in elements {
                            work.push(*elem);
                        }
                    }
                    KernelExprKind::Map(entries) => {
                        for entry in entries {
                            work.push(entry.key);
                            work.push(entry.value);
                        }
                    }
                    KernelExprKind::DomainMember(_)
                    | KernelExprKind::BuiltinClassMember(_)
                    | KernelExprKind::Builtin(_)
                    | KernelExprKind::SuffixedInteger(_) => {
                        errors.push(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            "the current Cranelift slice lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, niche and inline scalar Option constructors/carriers, record projection, inline-pipe gate plus straight-line transform/tap stages, scalar literals, static scalar tuple/record literals, Int/Bool arithmetic, Int/Float/Bool comparison, and native equality over scalar/Text/Bytes/record/tuple/scalar-Option/niche-Option shapes only",
                        ));
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(CodegenErrors::new(errors))
        }
    }

    fn compile(mut self) -> Result<CompiledProgram, CodegenErrors> {
        let kernel_ids = self
            .program
            .kernels()
            .iter()
            .filter(|(_, kernel)| !self.is_ambient_kernel(kernel))
            .map(|(kernel_id, _)| kernel_id)
            .collect::<Vec<_>>();
        let mut declaration_errors = Vec::new();
        for &kernel_id in &kernel_ids {
            let kernel = &self.program.kernels()[kernel_id];
            if let Err(error) = self.declare_kernel(kernel_id, kernel) {
                declaration_errors.push(error);
            }
        }
        if !declaration_errors.is_empty() {
            return Err(CodegenErrors::new(declaration_errors));
        }

        let mut compiled_kernels = Vec::with_capacity(kernel_ids.len());
        let mut errors = Vec::new();

        for &kernel_id in &kernel_ids {
            let kernel = &self.program.kernels()[kernel_id];
            match self.compile_kernel(kernel_id, kernel) {
                Ok(compiled) => compiled_kernels.push(compiled),
                Err(error) => errors.push(error),
            }
        }

        if !errors.is_empty() {
            return Err(CodegenErrors::new(errors));
        }

        let object = self.module.finish().emit().map_err(|error| {
            wrap_one(CodegenError::ObjectEmission {
                message: error.to_string().into_boxed_str(),
            })
        })?;
        let kernel_index = compiled_kernels
            .iter()
            .enumerate()
            .map(|(index, kernel)| (kernel.kernel, index))
            .collect();

        Ok(CompiledProgram {
            object,
            kernels: compiled_kernels,
            kernel_index,
        })
    }

    fn declare_kernel(&mut self, kernel_id: KernelId, kernel: &Kernel) -> Result<(), CodegenError> {
        let signature = self.build_signature(kernel_id, kernel)?;
        let symbol = kernel_symbol(self.program, kernel_id, kernel);
        let func_id = self
            .module
            .declare_function(&symbol, Linkage::Local, &signature)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: error.to_string().into_boxed_str(),
            })?;
        self.declared_functions.insert(kernel_id, func_id);
        Ok(())
    }

    fn compile_kernel(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
    ) -> Result<CompiledKernel, CodegenError> {
        match kernel.convention.kind {
            CallingConventionKind::RuntimeKernelV1 => {}
        }

        let symbol = kernel_symbol(self.program, kernel_id, kernel);
        let signature = self.build_signature(kernel_id, kernel)?;
        let func_id = *self
            .declared_functions
            .get(&kernel_id)
            .expect("declared kernels must be available before compilation");

        let mut ctx = self.module.make_context();
        ctx.func.signature = signature;
        ctx.func.name = UserFuncName::user(0, func_id.as_u32());
        let mut function_builder_ctx = std::mem::take(&mut self.function_builder_ctx);

        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut function_builder_ctx);
            let entry = builder.create_block();
            builder.switch_to_block(entry);
            builder.append_block_params_for_function_params(entry);

            let value = self.lower_kernel_body(kernel_id, kernel, &mut builder, entry)?;
            builder.ins().return_(&[value]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.function_builder_ctx = function_builder_ctx;

        if let Err(error) = verify_function(&ctx.func, self.module.isa()) {
            return Err(CodegenError::CraneliftVerifier {
                kernel: kernel_id,
                message: pretty_verifier_error(&ctx.func, None, error).into_boxed_str(),
            });
        }

        let clif = ctx.func.to_string().into_boxed_str();
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: error.to_string().into_boxed_str(),
            })?;
        let code_size = ctx
            .compiled_code()
            .map(|compiled| compiled.code_info().total_size as usize)
            .unwrap_or_default();

        Ok(CompiledKernel {
            kernel: kernel_id,
            symbol: symbol.into_boxed_str(),
            clif,
            code_size,
        })
    }

    fn build_signature(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
    ) -> Result<cranelift_codegen::ir::Signature, CodegenError> {
        let mut signature = self.module.make_signature();
        match kernel.convention.kind {
            CallingConventionKind::RuntimeKernelV1 => {}
        }
        for parameter in &kernel.convention.parameters {
            let ty = self.materialize_signature_type(
                kernel_id,
                parameter.layout,
                parameter.pass_mode,
                &format!("parameter {}", parameter.role),
            )?;
            signature.params.push(AbiParam::new(ty));
        }
        let result = self.materialize_signature_type(
            kernel_id,
            kernel.convention.result.layout,
            kernel.convention.result.pass_mode,
            "result",
        )?;
        signature.returns.push(AbiParam::new(result));
        Ok(signature)
    }

    fn lower_kernel_body(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
        builder: &mut FunctionBuilder<'_>,
        entry: cranelift_codegen::ir::Block,
    ) -> Result<Value, CodegenError> {
        enum Task {
            Visit(KernelExprId),
            BuildOptionSome(KernelExprId),
            BuildProjection(KernelExprId),
            BuildDirectApply {
                expr: KernelExprId,
                plan: DirectApplyPlan,
                argument_count: usize,
            },
            BuildUnary(KernelExprId),
            BuildBinary(KernelExprId),
            BuildRuntimeAggregate {
                expr_id: KernelExprId,
                count: usize,
            },
            BuildRuntimeText {
                expr_id: KernelExprId,
            },
            BuildRuntimeList {
                expr_id: KernelExprId,
                count: usize,
            },
            BuildRuntimeSet {
                expr_id: KernelExprId,
                count: usize,
            },
            BuildRuntimeMap {
                expr_id: KernelExprId,
                count: usize,
            },
            BuildPipeStage {
                pipe_expr: KernelExprId,
                stage_index: usize,
            },
            ContinuePipeGate {
                pipe_expr: KernelExprId,
                stage_index: usize,
                current: Value,
            },
            ContinuePipeTransform {
                pipe_expr: KernelExprId,
                stage_index: usize,
            },
            ContinuePipeTap {
                pipe_expr: KernelExprId,
                stage_index: usize,
                current: Value,
            },
            ContinuePipeTruthyFalsy {
                pipe_expr: KernelExprId,
                stage_index: usize,
                current: Value,
                merge_block: cranelift_codegen::ir::Block,
                falsy_block: cranelift_codegen::ir::Block,
            },
            FinalizePipeTruthyFalsy {
                pipe_expr: KernelExprId,
                stage_index: usize,
                merge_block: cranelift_codegen::ir::Block,
            },
            ContinuePipeCaseArmAfterBody {
                pipe_expr: KernelExprId,
                stage_index: usize,
                current: Value,
                arm_index: usize,
                merge_block: cranelift_codegen::ir::Block,
                next_block: Option<cranelift_codegen::ir::Block>,
            },
            RestoreInlineSubjects(Vec<(usize, Option<Value>)>),
            FinalizePipeFanOut {
                pipe_expr: KernelExprId,
                stage_index: usize,
                count: Value,
                result_array_ptr: Value,
                result_stride: u32,
                loop_header: cranelift_codegen::ir::Block,
                loop_exit: cranelift_codegen::ir::Block,
            },
        }

        fn snapshot_pipe_subjects(
            pipe: &crate::InlinePipeExpr,
            inline_subjects: &[Option<Value>],
        ) -> Vec<(usize, Option<Value>)> {
            let mut saved = Vec::new();
            for stage in &pipe.stages {
                for slot in [Some(stage.subject), stage.subject_memo, stage.result_memo]
                    .into_iter()
                    .flatten()
                {
                    let index = slot.index();
                    if saved.iter().all(|(saved_index, _)| *saved_index != index) {
                        saved.push((index, inline_subjects[index]));
                    }
                }
                match &stage.kind {
                    crate::InlinePipeStageKind::TruthyFalsy { truthy, falsy } => {
                        for slot in [truthy.payload_subject, falsy.payload_subject]
                            .into_iter()
                            .flatten()
                        {
                            let index = slot.index();
                            if saved.iter().all(|(saved_index, _)| *saved_index != index) {
                                saved.push((index, inline_subjects[index]));
                            }
                        }
                    }
                    crate::InlinePipeStageKind::Case { arms } => {
                        for arm in arms {
                            collect_pattern_binding_subjects(&arm.pattern, &mut |slot| {
                                let index = slot.index();
                                if saved.iter().all(|(saved_index, _)| *saved_index != index) {
                                    saved.push((index, inline_subjects[index]));
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
            saved
        }

        fn collect_pattern_binding_subjects(
            pattern: &crate::InlinePipePattern,
            callback: &mut impl FnMut(crate::InlineSubjectId),
        ) {
            match &pattern.kind {
                crate::InlinePipePatternKind::Binding { subject } => callback(*subject),
                crate::InlinePipePatternKind::Constructor { arguments, .. } => {
                    for p in arguments {
                        collect_pattern_binding_subjects(p, callback);
                    }
                }
                crate::InlinePipePatternKind::Tuple(pats) => {
                    for p in pats {
                        collect_pattern_binding_subjects(p, callback);
                    }
                }
                crate::InlinePipePatternKind::Record(fields) => {
                    for f in fields {
                        collect_pattern_binding_subjects(&f.pattern, callback);
                    }
                }
                crate::InlinePipePatternKind::List { elements, rest } => {
                    for p in elements {
                        collect_pattern_binding_subjects(p, callback);
                    }
                    if let Some(r) = rest {
                        collect_pattern_binding_subjects(r, callback);
                    }
                }
                _ => {}
            }
        }

        let mut input = None;
        let mut inline_subjects = vec![None; kernel.inline_subjects.len()];
        let mut environment = vec![None; kernel.environment.len()];
        let parameters = builder.block_params(entry).to_vec();
        for (value, parameter) in parameters.into_iter().zip(&kernel.convention.parameters) {
            match parameter.role {
                ParameterRole::InputSubject => input = Some(value),
                ParameterRole::Environment(slot) => environment[slot.index()] = Some(value),
            }
        }

        let mut tasks = vec![Task::Visit(kernel.root)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr_id) => {
                    let expr = &kernel.exprs()[expr_id];
                    if let Some(value) = self.materialize_static_expression_if_supported(
                        kernel_id, kernel, expr_id, builder,
                    )? {
                        values.push(value);
                        continue;
                    }
                    match &expr.kind {
                        KernelExprKind::Item(item) => {
                            match self.plan_item_reference(kernel_id, expr_id, *item)? {
                                ItemReferencePlan::DirectValue { body } => {
                                    values.push(self.lower_direct_item_call(
                                        kernel_id,
                                        body,
                                        &[],
                                        builder,
                                    )?);
                                }
                                ItemReferencePlan::SignalSlot { item } => {
                                    values.push(self.lower_signal_item_slot(
                                        kernel_id, expr_id, item, builder,
                                    )?);
                                }
                                ItemReferencePlan::ImportedSlot { item } => {
                                    values.push(self.lower_imported_item_slot(
                                        kernel_id, expr_id, item, builder,
                                    )?);
                                }
                                ItemReferencePlan::CallableDescriptor { item, body, arity } => {
                                    values.push(self.lower_item_callable_descriptor(
                                        kernel_id, item, body, arity, builder,
                                    )?);
                                }
                            }
                        }
                        KernelExprKind::Subject(subject) => {
                            let (value, _) = self.lower_subject_reference(
                                kernel_id,
                                kernel,
                                expr_id,
                                *subject,
                                input,
                                &inline_subjects,
                            )?;
                            values.push(value);
                        }
                        KernelExprKind::OptionSome { payload } => {
                            tasks.push(Task::BuildOptionSome(expr_id));
                            tasks.push(Task::Visit(*payload));
                        }
                        KernelExprKind::OptionNone => {
                            let contract = self.require_option_codegen_contract(
                                kernel_id,
                                kernel,
                                expr_id,
                                None,
                                expr.layout,
                                "None carrier",
                            )?;
                            let value = match contract {
                                OptionCodegenContract::NicheReference => {
                                    builder.ins().iconst(self.pointer_type(), 0)
                                }
                                OptionCodegenContract::InlineScalar(_) => {
                                    self.lower_inline_scalar_option_none(builder)
                                }
                            };
                            values.push(value);
                        }
                        KernelExprKind::Environment(slot) => {
                            let Some(Some(value)) = environment.get(slot.index()) else {
                                return Err(CodegenError::MissingEnvironmentParameter {
                                    kernel: kernel_id,
                                    slot: *slot,
                                });
                            };
                            values.push(*value);
                        }
                        KernelExprKind::Integer(integer) => {
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "integer literal",
                            )?;
                            let raw = integer.raw.as_ref();
                            let value = raw.parse::<i64>().map_err(|_| {
                                CodegenError::InvalidIntegerLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: integer.raw.clone(),
                                }
                            })?;
                            values.push(builder.ins().iconst(types::I64, value));
                        }
                        KernelExprKind::Float(float) => {
                            self.require_float_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "Float literal",
                            )?;
                            let value = RuntimeFloat::parse_literal(float.raw.as_ref()).ok_or(
                                CodegenError::InvalidFloatLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: float.raw.clone(),
                                },
                            )?;
                            values.push(builder.ins().f64const(Ieee64::with_float(value.to_f64())));
                        }
                        KernelExprKind::Decimal(decimal) => {
                            self.require_decimal_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "Decimal literal",
                            )?;
                            let value = RuntimeDecimal::parse_literal(decimal.raw.as_ref()).ok_or(
                                CodegenError::InvalidDecimalLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: decimal.raw.clone(),
                                },
                            )?;
                            values.push(self.materialize_literal_pointer(
                                kernel_id,
                                "decimal_literal",
                                value.encode_constant_bytes(),
                                16,
                                builder,
                            )?);
                        }
                        KernelExprKind::BigInt(bigint) => {
                            self.require_bigint_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "BigInt literal",
                            )?;
                            let value = RuntimeBigInt::parse_literal(bigint.raw.as_ref()).ok_or(
                                CodegenError::InvalidBigIntLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: bigint.raw.clone(),
                                },
                            )?;
                            values.push(self.materialize_literal_pointer(
                                kernel_id,
                                "bigint_literal",
                                value.encode_constant_bytes(),
                                8,
                                builder,
                            )?);
                        }
                        KernelExprKind::Text(text) => {
                            self.require_text_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "Text literal",
                            )?;
                            // Try static first
                            if let Ok(Some(rendered)) =
                                self.render_static_text_literal(kernel_id, kernel, expr_id, text)
                            {
                                values.push(self.materialize_text_constant(
                                    kernel_id,
                                    rendered.as_ref(),
                                    builder,
                                )?);
                            } else {
                                // Dynamic: visit interpolation sub-expressions in reverse
                                tasks.push(Task::BuildRuntimeText { expr_id });
                                for segment in text.segments.iter().rev() {
                                    if let crate::TextSegment::Interpolation {
                                        expr: interp_expr,
                                        ..
                                    } = segment
                                    {
                                        tasks.push(Task::Visit(*interp_expr));
                                    }
                                }
                            }
                        }
                        KernelExprKind::Tuple(elements) => {
                            // Try static materialization first; fall back to runtime aggregate
                            if let Ok(value) = self.materialize_static_scalar_aggregate_expression(
                                kernel_id, kernel, expr_id, builder,
                            ) {
                                values.push(value);
                            } else {
                                tasks.push(Task::BuildRuntimeAggregate {
                                    expr_id,
                                    count: elements.len(),
                                });
                                for elem in elements.iter().rev() {
                                    tasks.push(Task::Visit(*elem));
                                }
                            }
                        }
                        KernelExprKind::Record(fields) => {
                            // Try static materialization first; fall back to runtime aggregate
                            if let Ok(value) = self.materialize_static_scalar_aggregate_expression(
                                kernel_id, kernel, expr_id, builder,
                            ) {
                                values.push(value);
                            } else {
                                tasks.push(Task::BuildRuntimeAggregate {
                                    expr_id,
                                    count: fields.len(),
                                });
                                for field in fields.iter().rev() {
                                    tasks.push(Task::Visit(field.value));
                                }
                            }
                        }
                        KernelExprKind::List(elements) => {
                            tasks.push(Task::BuildRuntimeList {
                                expr_id,
                                count: elements.len(),
                            });
                            for elem in elements.iter().rev() {
                                tasks.push(Task::Visit(*elem));
                            }
                        }
                        KernelExprKind::Set(elements) => {
                            tasks.push(Task::BuildRuntimeSet {
                                expr_id,
                                count: elements.len(),
                            });
                            for elem in elements.iter().rev() {
                                tasks.push(Task::Visit(*elem));
                            }
                        }
                        KernelExprKind::Map(entries) => {
                            tasks.push(Task::BuildRuntimeMap {
                                expr_id,
                                count: entries.len(),
                            });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(entry.value));
                                tasks.push(Task::Visit(entry.key));
                            }
                        }
                        KernelExprKind::Builtin(BuiltinTerm::True) => {
                            self.require_bool_expression(kernel_id, expr_id, expr.layout, "True")?;
                            values.push(builder.ins().iconst(types::I8, 1));
                        }
                        KernelExprKind::Builtin(BuiltinTerm::False) => {
                            self.require_bool_expression(kernel_id, expr_id, expr.layout, "False")?;
                            values.push(builder.ins().iconst(types::I8, 0));
                        }
                        KernelExprKind::Builtin(BuiltinTerm::None) => {
                            let contract = self.require_option_codegen_contract(
                                kernel_id,
                                kernel,
                                expr_id,
                                None,
                                expr.layout,
                                "None constructor",
                            )?;
                            let value = match contract {
                                OptionCodegenContract::NicheReference => {
                                    builder.ins().iconst(self.pointer_type(), 0)
                                }
                                OptionCodegenContract::InlineScalar(_) => {
                                    self.lower_inline_scalar_option_none(builder)
                                }
                            };
                            values.push(value);
                        }
                        KernelExprKind::IntrinsicValue(intrinsic) => {
                            values.push(self.lower_intrinsic_value(
                                kernel_id,
                                expr_id,
                                *intrinsic,
                                expr.layout,
                                builder,
                            )?);
                        }
                        KernelExprKind::Projection { base, .. } => match base {
                            crate::ProjectionBase::Subject(subject) => {
                                let (value, base_layout) = self.lower_subject_reference(
                                    kernel_id,
                                    kernel,
                                    expr_id,
                                    *subject,
                                    input,
                                    &inline_subjects,
                                )?;
                                values.push(self.lower_projection(
                                    kernel_id,
                                    kernel,
                                    expr_id,
                                    value,
                                    base_layout,
                                    builder,
                                )?);
                            }
                            crate::ProjectionBase::Expr(base) => {
                                tasks.push(Task::BuildProjection(expr_id));
                                tasks.push(Task::Visit(*base));
                            }
                        },
                        KernelExprKind::Unary { expr, .. } => {
                            tasks.push(Task::BuildUnary(expr_id));
                            tasks.push(Task::Visit(*expr));
                        }
                        KernelExprKind::Binary { left, right, .. } => {
                            tasks.push(Task::BuildBinary(expr_id));
                            tasks.push(Task::Visit(*right));
                            tasks.push(Task::Visit(*left));
                        }
                        KernelExprKind::Apply { callee, arguments } => {
                            let plan = self.resolve_direct_apply_plan(
                                kernel_id, expr_id, *callee, arguments,
                            )?;
                            tasks.push(Task::BuildDirectApply {
                                expr: expr_id,
                                plan,
                                argument_count: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(*argument));
                            }
                        }
                        KernelExprKind::Pipe(pipe) => {
                            let saved = snapshot_pipe_subjects(pipe, &inline_subjects);
                            if !saved.is_empty() {
                                tasks.push(Task::RestoreInlineSubjects(saved));
                            }
                            if !pipe.stages.is_empty() {
                                tasks.push(Task::BuildPipeStage {
                                    pipe_expr: expr_id,
                                    stage_index: 0,
                                });
                            }
                            tasks.push(Task::Visit(pipe.head));
                        }
                        KernelExprKind::SumConstructor(handle) => {
                            if handle.field_count != 0 {
                                return Err(self.unsupported_expression(
                                    kernel_id,
                                    expr_id,
                                    &format!(
                                        "sum constructor `{}.{}` with {} field(s) cannot be used as a standalone value; use it as a callee in an Apply expression",
                                        handle.type_name, handle.variant_name, handle.field_count
                                    ),
                                ));
                            }
                            let tag = match &self.program.layouts()[expr.layout].kind {
                                LayoutKind::Sum(variants) => variants
                                    .iter()
                                    .position(|v| v.name.as_ref() == handle.variant_name.as_ref())
                                    .unwrap_or(0)
                                    as i64,
                                LayoutKind::Opaque { .. } | LayoutKind::Domain { .. } => {
                                    sum_variant_tag_for_opaque(handle.variant_name.as_ref())
                                }
                                _ => {
                                    return Err(self.unsupported_expression(
                                        kernel_id,
                                        expr_id,
                                        "sum constructor requires a Sum, Opaque, or Domain layout",
                                    ));
                                }
                            };
                            let tag_bytes: Box<[u8]> =
                                tag.to_le_bytes().to_vec().into_boxed_slice();
                            values.push(self.materialize_literal_pointer(
                                kernel_id,
                                "sum_singleton",
                                tag_bytes,
                                8,
                                builder,
                            )?);
                        }
                        _ => {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "the current Cranelift slice only lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, niche and inline scalar Option constructors/carriers, record projection, scalar subjects/environment slots, inline-pipe gate plus straight-line transform/tap stages, scalar literals, static scalar tuple/record literals, Int/Bool arithmetic, Int/Float/Bool comparison, and native equality over scalar/Text/Bytes/record/tuple/scalar-Option/niche-Option shapes",
                            ));
                        }
                    }
                }
                Task::BuildOptionSome(expr_id) => {
                    let expr = &kernel.exprs()[expr_id];
                    let value = values.pop().expect("option payload should exist");
                    let KernelExprKind::OptionSome { payload } = &expr.kind else {
                        unreachable!("build task must only be queued for option expressions");
                    };
                    let contract = self.require_option_codegen_contract(
                        kernel_id,
                        kernel,
                        expr_id,
                        Some(*payload),
                        expr.layout,
                        "Some carrier",
                    )?;
                    let value = match contract {
                        OptionCodegenContract::NicheReference => value,
                        OptionCodegenContract::InlineScalar(kind) => {
                            self.lower_inline_scalar_option_some(kind, value, builder)
                        }
                    };
                    values.push(value);
                }
                Task::BuildProjection(expr_id) => {
                    let expr = &kernel.exprs()[expr_id];
                    let base = values.pop().expect("projection base should exist");
                    let KernelExprKind::Projection {
                        base: base_kind, ..
                    } = &expr.kind
                    else {
                        unreachable!("build task must only be queued for projection expressions");
                    };
                    let crate::ProjectionBase::Expr(base_expr) = base_kind else {
                        unreachable!(
                            "projection build task should only be queued for expression-based bases"
                        );
                    };
                    values.push(self.lower_projection(
                        kernel_id,
                        kernel,
                        expr_id,
                        base,
                        kernel.exprs()[*base_expr].layout,
                        builder,
                    )?);
                }
                Task::BuildDirectApply {
                    expr,
                    plan,
                    argument_count,
                } => {
                    let mut argument_values = Vec::with_capacity(argument_count);
                    for _ in 0..argument_count {
                        argument_values
                            .push(values.pop().expect("direct apply argument should exist"));
                    }
                    argument_values.reverse();
                    values.push(self.lower_direct_apply(
                        kernel_id,
                        expr,
                        plan,
                        &argument_values,
                        builder,
                    )?);
                }
                Task::BuildUnary(expr_id) => {
                    let expr = &kernel.exprs()[expr_id];
                    let value = values.pop().expect("unary child should exist");
                    let KernelExprKind::Unary {
                        operator,
                        expr: inner,
                    } = &expr.kind
                    else {
                        unreachable!("build task must only be queued for unary expressions");
                    };
                    let lowered = match operator {
                        UnaryOperator::Not => {
                            self.require_bool_expression(
                                kernel_id,
                                *inner,
                                kernel.exprs()[*inner].layout,
                                "logical not operand",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "logical not result",
                            )?;
                            let one = builder.ins().iconst(types::I8, 1);
                            builder.ins().bxor(value, one)
                        }
                    };
                    values.push(lowered);
                }
                Task::BuildBinary(expr_id) => {
                    let rhs = values.pop().expect("binary rhs should exist");
                    let lhs = values.pop().expect("binary lhs should exist");
                    let expr = &kernel.exprs()[expr_id];
                    let KernelExprKind::Binary {
                        left,
                        operator,
                        right,
                    } = &expr.kind
                    else {
                        unreachable!("build task must only be queued for binary expressions");
                    };
                    let lowered = match operator {
                        BinaryOperator::Add => {
                            match self.require_arithmetic_expression_triple(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeArithmeticKind::Integer => builder.ins().iadd(lhs, rhs),
                                NativeArithmeticKind::Decimal => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_decimal_add", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeArithmeticKind::BigInt => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_bigint_add", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::Subtract => {
                            match self.require_arithmetic_expression_triple(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeArithmeticKind::Integer => builder.ins().isub(lhs, rhs),
                                NativeArithmeticKind::Decimal => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_decimal_sub", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeArithmeticKind::BigInt => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_bigint_sub", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::Multiply => {
                            match self.require_arithmetic_expression_triple(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeArithmeticKind::Integer => builder.ins().imul(lhs, rhs),
                                NativeArithmeticKind::Decimal => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_decimal_mul", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeArithmeticKind::BigInt => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_bigint_mul", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::Divide => {
                            match self.require_arithmetic_expression_triple(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeArithmeticKind::Integer => builder.ins().sdiv(lhs, rhs),
                                NativeArithmeticKind::Decimal => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_decimal_div", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeArithmeticKind::BigInt => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_bigint_div", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::Modulo => {
                            match self.require_arithmetic_expression_triple(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeArithmeticKind::Integer => builder.ins().srem(lhs, rhs),
                                NativeArithmeticKind::Decimal => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_decimal_mod", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeArithmeticKind::BigInt => {
                                    let func_ref = self.declare_ptr_binop_func(
                                        "aivi_bigint_mod", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::GreaterThan => {
                            match self.require_ordered_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeCompareKind::Integer => {
                                    builder.ins().icmp(IntCC::SignedGreaterThan, lhs, rhs)
                                }
                                NativeCompareKind::Float => {
                                    builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs)
                                }
                                NativeCompareKind::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_gt", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeCompareKind::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_gt", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::LessThan => {
                            match self.require_ordered_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeCompareKind::Integer => {
                                    builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs)
                                }
                                NativeCompareKind::Float => {
                                    builder.ins().fcmp(FloatCC::LessThan, lhs, rhs)
                                }
                                NativeCompareKind::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_lt", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeCompareKind::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_lt", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::GreaterThanOrEqual => {
                            match self.require_ordered_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeCompareKind::Integer => {
                                    builder
                                        .ins()
                                        .icmp(IntCC::SignedGreaterThanOrEqual, lhs, rhs)
                                }
                                NativeCompareKind::Float => {
                                    builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs)
                                }
                                NativeCompareKind::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_gte", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeCompareKind::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_gte", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::LessThanOrEqual => {
                            match self.require_ordered_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeCompareKind::Integer => {
                                    builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs, rhs)
                                }
                                NativeCompareKind::Float => {
                                    builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs)
                                }
                                NativeCompareKind::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_lte", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeCompareKind::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_lte", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                            }
                        }
                        BinaryOperator::Equals => {
                            let shape = self.require_equatable_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )?;
                            match &shape {
                                NativeEqualityShape::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_eq", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                NativeEqualityShape::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_eq", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    builder.inst_results(call)[0]
                                }
                                _ => self.lower_native_equality_shape(
                                    kernel_id, expr_id, &shape, lhs, rhs, builder,
                                )?,
                            }
                        }
                        BinaryOperator::NotEquals => {
                            let shape = self.require_equatable_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )?;
                            match &shape {
                                NativeEqualityShape::Integer => {
                                    builder.ins().icmp(IntCC::NotEqual, lhs, rhs)
                                }
                                NativeEqualityShape::Float => {
                                    builder.ins().fcmp(FloatCC::NotEqual, lhs, rhs)
                                }
                                NativeEqualityShape::Decimal => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_decimal_eq", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    let equal = builder.inst_results(call)[0];
                                    let one = builder.ins().iconst(types::I8, 1);
                                    builder.ins().bxor(equal, one)
                                }
                                NativeEqualityShape::BigInt => {
                                    let func_ref = self.declare_ptr_cmp_func(
                                        "aivi_bigint_eq", kernel_id, builder,
                                    )?;
                                    let call = builder.ins().call(func_ref, &[lhs, rhs]);
                                    let equal = builder.inst_results(call)[0];
                                    let one = builder.ins().iconst(types::I8, 1);
                                    builder.ins().bxor(equal, one)
                                }
                                NativeEqualityShape::Text
                                | NativeEqualityShape::Bytes
                                | NativeEqualityShape::Aggregate(_)
                                | NativeEqualityShape::InlineScalarOption(_)
                                | NativeEqualityShape::NicheOption { .. } => {
                                    let equal = self.lower_native_equality_shape(
                                        kernel_id, expr_id, &shape, lhs, rhs, builder,
                                    )?;
                                    let one = builder.ins().iconst(types::I8, 1);
                                    builder.ins().bxor(equal, one)
                                }
                            }
                        }
                        BinaryOperator::And => {
                            self.require_bool_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "logical and lhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "logical and rhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "logical and result",
                            )?;
                            builder.ins().band(lhs, rhs)
                        }
                        BinaryOperator::Or => {
                            self.require_bool_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "logical or lhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "logical or rhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "logical or result",
                            )?;
                            builder.ins().bor(lhs, rhs)
                        }
                    };
                    values.push(lowered);
                }
                Task::BuildPipeStage {
                    pipe_expr,
                    stage_index,
                } => {
                    let current = values.pop().expect("pipe stage input should exist");
                    let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                    let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                        unreachable!("pipe build task must only be queued for pipe expressions");
                    };
                    let stage = &pipe.stages[stage_index];
                    let current_layout = if stage_index == 0 {
                        kernel.exprs()[pipe.head].layout
                    } else {
                        pipe.stages[stage_index - 1].result_layout
                    };
                    self.require_layout_match(
                        kernel_id,
                        pipe_expr,
                        stage.input_layout,
                        current_layout,
                        &format!("inline-pipe stage {stage_index} input"),
                    )?;
                    inline_subjects[stage.subject.index()] = Some(current);
                    if let Some(slot) = stage.subject_memo {
                        inline_subjects[slot.index()] = Some(current);
                    }
                    match &stage.kind {
                        crate::InlinePipeStageKind::Transform { expr, .. } => {
                            tasks.push(Task::ContinuePipeTransform {
                                pipe_expr,
                                stage_index,
                            });
                            tasks.push(Task::Visit(*expr));
                        }
                        crate::InlinePipeStageKind::Tap { expr } => {
                            tasks.push(Task::ContinuePipeTap {
                                pipe_expr,
                                stage_index,
                                current,
                            });
                            tasks.push(Task::Visit(*expr));
                        }
                        crate::InlinePipeStageKind::Gate { predicate, .. } => {
                            tasks.push(Task::ContinuePipeGate {
                                pipe_expr,
                                stage_index,
                                current,
                            });
                            tasks.push(Task::Visit(*predicate));
                        }
                        crate::InlinePipeStageKind::Debug { .. } => {
                            // Debug is a no-op in compiled code: pass through input unchanged.
                            self.require_layout_match(
                                kernel_id,
                                pipe_expr,
                                stage.result_layout,
                                stage.input_layout,
                                &format!("inline-pipe debug stage {stage_index} result"),
                            )?;
                            if let Some(slot) = stage.result_memo {
                                inline_subjects[slot.index()] = Some(current);
                            }
                            values.push(current);
                            if stage_index + 1 < pipe.stages.len() {
                                tasks.push(Task::BuildPipeStage {
                                    pipe_expr,
                                    stage_index: stage_index + 1,
                                });
                            }
                        }
                        crate::InlinePipeStageKind::Case { arms } => {
                            if arms.is_empty() {
                                return Err(self.unsupported_inline_pipe_stage(
                                    kernel_id,
                                    pipe_expr,
                                    stage_index,
                                    "empty Case arms",
                                ));
                            }
                            let result_abi = self.field_abi_shape(
                                kernel_id,
                                stage.result_layout,
                                "case result",
                            )?;
                            let merge_block = builder.create_block();
                            builder.append_block_param(merge_block, result_abi.ty);

                            let first_arm_body = arms[0].body;
                            let first_arm_pattern = arms[0].pattern.clone();
                            let arm_body_block = builder.create_block();
                            let next_block = if arms.len() > 1 {
                                Some(builder.create_block())
                            } else {
                                None
                            };
                            // When there is no next arm, branch to a trap block (exhaustive match)
                            let false_target = next_block.unwrap_or_else(|| builder.create_block());
                            let cond = self.emit_pattern_test(
                                kernel_id,
                                current,
                                &first_arm_pattern,
                                stage.input_layout,
                                &mut inline_subjects,
                                builder,
                            )?;
                            builder
                                .ins()
                                .brif(cond, arm_body_block, &[], false_target, &[]);
                            if next_block.is_none() {
                                builder.switch_to_block(false_target);
                                builder
                                    .ins()
                                    .trap(cranelift_codegen::ir::TrapCode::STACK_OVERFLOW);
                            }
                            builder.switch_to_block(arm_body_block);
                            self.apply_pattern_bindings(
                                kernel_id,
                                current,
                                &first_arm_pattern,
                                stage.input_layout,
                                &mut inline_subjects,
                                builder,
                            );
                            tasks.push(Task::ContinuePipeCaseArmAfterBody {
                                pipe_expr,
                                stage_index,
                                current,
                                arm_index: 0,
                                merge_block,
                                next_block,
                            });
                            tasks.push(Task::Visit(first_arm_body));
                        }
                        crate::InlinePipeStageKind::TruthyFalsy { truthy, falsy: _ } => {
                            let result_abi = self.field_abi_shape(
                                kernel_id,
                                stage.result_layout,
                                "truthy-falsy result",
                            )?;
                            let truthy_block = builder.create_block();
                            let falsy_block = builder.create_block();
                            let merge_block = builder.create_block();
                            builder.append_block_param(merge_block, result_abi.ty);

                            let truthy_constructor = truthy.constructor.clone();
                            let truthy_payload_subject = truthy.payload_subject;
                            let truthy_body = truthy.body;
                            let cond = self.emit_truthy_falsy_condition(
                                kernel_id,
                                pipe_expr,
                                stage_index,
                                current,
                                stage.input_layout,
                                &truthy_constructor,
                                builder,
                            )?;
                            builder
                                .ins()
                                .brif(cond, truthy_block, &[], falsy_block, &[]);

                            builder.switch_to_block(truthy_block);
                            if let Some(slot) = truthy_payload_subject {
                                let payload = self.extract_truthy_falsy_payload(
                                    current,
                                    stage.input_layout,
                                    &truthy_constructor,
                                    builder,
                                );
                                inline_subjects[slot.index()] = Some(payload);
                            }
                            tasks.push(Task::ContinuePipeTruthyFalsy {
                                pipe_expr,
                                stage_index,
                                current,
                                merge_block,
                                falsy_block,
                            });
                            tasks.push(Task::Visit(truthy_body));
                        }
                        crate::InlinePipeStageKind::FanOut { map_expr } => {
                            // Fan-out: iterate list, apply map_expr to each element,
                            // collect results into a new list.
                            let input_layout = stage.input_layout;
                            let result_layout = stage.result_layout;
                            let LayoutKind::List { element: input_elem } =
                                &self.program.layouts()[input_layout].kind.clone()
                            else {
                                return Err(self.unsupported_inline_pipe_stage(
                                    kernel_id,
                                    pipe_expr,
                                    stage_index,
                                    "fan-out input must be List",
                                ));
                            };
                            let LayoutKind::List { element: result_elem } =
                                &self.program.layouts()[result_layout].kind.clone()
                            else {
                                return Err(self.unsupported_inline_pipe_stage(
                                    kernel_id,
                                    pipe_expr,
                                    stage_index,
                                    "fan-out result must be List",
                                ));
                            };
                            let _input_elem_abi =
                                self.field_abi_shape(kernel_id, *input_elem, "fanout input element")?;
                            let result_elem_abi =
                                self.field_abi_shape(kernel_id, *result_elem, "fanout result element")?;
                            let result_stride = result_elem_abi.size.max(1);

                            // Get list length
                            let list_len_func =
                                self.declare_list_len_func(kernel_id, builder)?;
                            let len_call =
                                builder.ins().call(list_len_func, &[current]);
                            let count =
                                builder.inst_results(len_call)[0];

                            // Allocate result array (count * stride, minimum 8 bytes)
                            // Use a generous fixed upper bound for the stack slot;
                            // the actual iteration uses count at runtime.
                            let max_static_slots = 64u32;
                            let array_size = max_static_slots * result_stride;
                            let array_slot = builder.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    array_size.max(8),
                                    result_elem_abi.align.max(1).ilog2() as u8,
                                ),
                            );
                            let result_array_ptr =
                                builder.ins().stack_addr(self.pointer_type(), array_slot, 0);

                            // Create loop blocks
                            let loop_header = builder.create_block();
                            let loop_body = builder.create_block();
                            let loop_exit = builder.create_block();

                            // loop_header takes the counter as block param
                            builder.append_block_param(loop_header, types::I64);

                            // Jump to loop header with counter = 0
                            let zero = builder.ins().iconst(types::I64, 0);
                            builder.ins().jump(loop_header, &[BlockArg::Value(zero)]);

                            // Loop header: check counter < count
                            builder.switch_to_block(loop_header);
                            let counter = builder.block_params(loop_header)[0];
                            let cond =
                                builder
                                    .ins()
                                    .icmp(IntCC::SignedLessThan, counter, count);
                            builder.ins().brif(
                                cond,
                                loop_body,
                                &[],
                                loop_exit,
                                &[],
                            );

                            // Loop body: get element, set subject, evaluate map_expr
                            builder.switch_to_block(loop_body);
                            let list_get_func =
                                self.declare_list_get_func(kernel_id, builder)?;
                            let get_call = builder.ins().call(
                                list_get_func,
                                &[current, counter],
                            );
                            let element = builder.inst_results(get_call)[0];

                            // Set the stage subject to the element
                            inline_subjects[stage.subject.index()] = Some(element);

                            // Push finalization task, then visit map_expr
                            tasks.push(Task::FinalizePipeFanOut {
                                pipe_expr,
                                stage_index,
                                count,
                                result_array_ptr,
                                result_stride,
                                loop_header,
                                loop_exit,
                            });
                            tasks.push(Task::Visit(*map_expr));
                        }
                    }
                }
                Task::ContinuePipeGate {
                    pipe_expr,
                    stage_index,
                    current,
                } => {
                    let predicate = values
                        .pop()
                        .expect("pipe gate predicate result should exist");
                    let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                    let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                        unreachable!("pipe continuation must only be queued for pipe expressions");
                    };
                    let stage = &pipe.stages[stage_index];
                    let crate::InlinePipeStageKind::Gate {
                        predicate: predicate_expr,
                        ..
                    } = &stage.kind
                    else {
                        unreachable!("gate continuation must only be queued for gate stages");
                    };
                    self.require_bool_expression(
                        kernel_id,
                        *predicate_expr,
                        kernel.exprs()[*predicate_expr].layout,
                        &format!("inline-pipe stage {stage_index} predicate"),
                    )?;
                    let contract = self.require_inline_pipe_gate_contract(
                        kernel_id,
                        pipe_expr,
                        stage_index,
                        stage.input_layout,
                        stage.result_layout,
                    )?;
                    let result = self.lower_inline_pipe_gate(contract, current, predicate, builder);
                    if let Some(slot) = stage.result_memo {
                        inline_subjects[slot.index()] = Some(result);
                    }
                    values.push(result);
                    if stage_index + 1 < pipe.stages.len() {
                        tasks.push(Task::BuildPipeStage {
                            pipe_expr,
                            stage_index: stage_index + 1,
                        });
                    }
                }
                Task::ContinuePipeTransform {
                    pipe_expr,
                    stage_index,
                } => {
                    let result = values.pop().expect("pipe transform result should exist");
                    let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                    let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                        unreachable!("pipe continuation must only be queued for pipe expressions");
                    };
                    let stage = &pipe.stages[stage_index];
                    let crate::InlinePipeStageKind::Transform { expr, .. } = &stage.kind else {
                        unreachable!(
                            "transform continuation must only be queued for transform stages"
                        );
                    };
                    self.require_layout_match(
                        kernel_id,
                        pipe_expr,
                        stage.result_layout,
                        kernel.exprs()[*expr].layout,
                        &format!("inline-pipe stage {stage_index} result"),
                    )?;
                    if let Some(slot) = stage.result_memo {
                        inline_subjects[slot.index()] = Some(result);
                    }
                    values.push(result);
                    if stage_index + 1 < pipe.stages.len() {
                        tasks.push(Task::BuildPipeStage {
                            pipe_expr,
                            stage_index: stage_index + 1,
                        });
                    }
                }
                Task::ContinuePipeTap {
                    pipe_expr,
                    stage_index,
                    current,
                } => {
                    let _ = values.pop().expect("pipe tap result should exist");
                    let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                    let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                        unreachable!("pipe continuation must only be queued for pipe expressions");
                    };
                    let stage = &pipe.stages[stage_index];
                    let crate::InlinePipeStageKind::Tap { .. } = &stage.kind else {
                        unreachable!("tap continuation must only be queued for tap stages");
                    };
                    self.require_layout_match(
                        kernel_id,
                        pipe_expr,
                        stage.result_layout,
                        stage.input_layout,
                        &format!("inline-pipe tap stage {stage_index} result"),
                    )?;
                    if let Some(slot) = stage.result_memo {
                        inline_subjects[slot.index()] = Some(current);
                    }
                    values.push(current);
                    if stage_index + 1 < pipe.stages.len() {
                        tasks.push(Task::BuildPipeStage {
                            pipe_expr,
                            stage_index: stage_index + 1,
                        });
                    }
                }
                Task::RestoreInlineSubjects(saved) => {
                    for (index, value) in saved {
                        inline_subjects[index] = value;
                    }
                }
                Task::FinalizePipeFanOut {
                    pipe_expr,
                    stage_index,
                    count,
                    result_array_ptr,
                    result_stride,
                    loop_header,
                    loop_exit,
                } => {
                    // map_expr result is on the values stack
                    let map_result = values.pop().expect("fanout map result");

                    let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                    let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                        unreachable!();
                    };
                    let stage = &pipe.stages[stage_index];

                    // Store mapped element at result_array[counter * stride]
                    let counter = builder.block_params(loop_header)[0];
                    let offset = builder.ins().imul_imm(counter, result_stride as i64);
                    let dest = builder.ins().iadd(result_array_ptr, offset);
                    builder.ins().store(MemFlags::new(), map_result, dest, 0);

                    // Increment counter and jump back to loop header
                    let next_counter = builder.ins().iadd_imm(counter, 1);
                    builder.ins().jump(loop_header, &[BlockArg::Value(next_counter)]);

                    // Exit block: construct result list
                    builder.switch_to_block(loop_exit);
                    let list_new_func =
                        self.declare_list_new_func(kernel_id, builder)?;
                    let stride_val =
                        builder.ins().iconst(types::I64, result_stride as i64);
                    let new_list_call = builder.ins().call(
                        list_new_func,
                        &[count, result_array_ptr, stride_val],
                    );
                    let result_list = builder.inst_results(new_list_call)[0];

                    if let Some(slot) = stage.result_memo {
                        inline_subjects[slot.index()] = Some(result_list);
                    }
                    values.push(result_list);
                    if stage_index + 1 < pipe.stages.len() {
                        tasks.push(Task::BuildPipeStage {
                            pipe_expr,
                            stage_index: stage_index + 1,
                        });
                    }
                }
                Task::BuildRuntimeAggregate { expr_id, count } => {
                    let layout = kernel.exprs()[expr_id].layout;
                    let field_layouts: Vec<LayoutId> =
                        match &self.program.layouts()[layout].kind.clone() {
                            LayoutKind::Tuple(elements) => elements.clone(),
                            LayoutKind::Record(fields) => fields.iter().map(|f| f.layout).collect(),
                            _ => {
                                return Err(self.unsupported_expression(
                                    kernel_id,
                                    expr_id,
                                    "BuildRuntimeAggregate requires Tuple or Record layout",
                                ));
                            }
                        };
                    let mut field_values: Vec<Value> = (0..count)
                        .map(|_| values.pop().expect("aggregate field value"))
                        .collect();
                    field_values.reverse();

                    let mut total_size = 0u32;
                    let mut max_align = 1u32;
                    let mut offsets: Vec<u32> = Vec::new();
                    for &field_layout in &field_layouts {
                        let abi =
                            self.field_abi_shape(kernel_id, field_layout, "aggregate field")?;
                        max_align = max_align.max(abi.align);
                        total_size = align_to(total_size, abi.align);
                        offsets.push(total_size);
                        total_size += abi.size;
                    }
                    if max_align > 0 {
                        total_size = align_to(total_size, max_align);
                    }
                    if total_size == 0 {
                        total_size = 1;
                    }

                    let slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            total_size,
                            max_align.ilog2() as u8,
                        ));
                    let base = builder.ins().stack_addr(self.pointer_type(), slot, 0);

                    for (i, &offset) in offsets.iter().enumerate() {
                        builder
                            .ins()
                            .store(MemFlags::new(), field_values[i], base, offset as i32);
                    }
                    values.push(base);
                }
                Task::BuildRuntimeText { expr_id } => {
                    let text_segments = {
                        let expr = &kernel.exprs()[expr_id];
                        let KernelExprKind::Text(text) = &expr.kind else {
                            unreachable!()
                        };
                        text.segments.clone()
                    };

                    let n_interps = text_segments
                        .iter()
                        .filter(|s| matches!(s, crate::TextSegment::Interpolation { .. }))
                        .count();

                    let mut interp_values: Vec<Value> = (0..n_interps)
                        .map(|_| values.pop().expect("text interp value"))
                        .collect();
                    interp_values.reverse();

                    let mut seg_values: Vec<Value> = Vec::with_capacity(text_segments.len());
                    let mut interp_iter = interp_values.into_iter();
                    for segment in &text_segments {
                        let v = match segment {
                            crate::TextSegment::Fragment { raw, .. } => {
                                self.materialize_text_constant(kernel_id, raw.as_ref(), builder)?
                            }
                            crate::TextSegment::Interpolation { .. } => {
                                interp_iter.next().expect("interpolation value")
                            }
                        };
                        seg_values.push(v);
                    }

                    let n_segs = seg_values.len() as u32;
                    let array_size = n_segs * 8;
                    let array_slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            array_size.max(8),
                            3,
                        ));
                    let array_ptr = builder.ins().stack_addr(self.pointer_type(), array_slot, 0);
                    for (i, seg_val) in seg_values.iter().enumerate() {
                        builder
                            .ins()
                            .store(MemFlags::new(), *seg_val, array_ptr, (i * 8) as i32);
                    }

                    let concat_func = self.declare_text_concat_func(kernel_id, builder)?;
                    let count_val = builder.ins().iconst(types::I64, n_segs as i64);
                    let call = builder.ins().call(concat_func, &[count_val, array_ptr]);
                    let result = builder.inst_results(call)[0];
                    values.push(result);
                }
                Task::BuildRuntimeList { expr_id, count } => {
                    let mut element_values: Vec<Value> = (0..count)
                        .map(|_| values.pop().expect("list element value"))
                        .collect();
                    element_values.reverse();

                    let elem_abi = {
                        let layout = kernel.exprs()[expr_id].layout;
                        let LayoutKind::List { element } =
                            &self.program.layouts()[layout].kind.clone()
                        else {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "BuildRuntimeList requires List layout",
                            ));
                        };
                        self.field_abi_shape(kernel_id, *element, "list element")?
                    };
                    let stride = elem_abi.size.max(1);
                    let array_size = (count as u32) * stride;
                    let array_slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            array_size.max(8),
                            elem_abi.align.max(1).ilog2() as u8,
                        ));
                    let array_ptr = builder.ins().stack_addr(self.pointer_type(), array_slot, 0);
                    for (i, elem_val) in element_values.iter().enumerate() {
                        builder.ins().store(
                            MemFlags::new(),
                            *elem_val,
                            array_ptr,
                            (i as u32 * stride) as i32,
                        );
                    }

                    let list_func = self.declare_list_new_func(kernel_id, builder)?;
                    let count_val = builder.ins().iconst(types::I64, count as i64);
                    let stride_val = builder.ins().iconst(types::I64, stride as i64);
                    let call =
                        builder
                            .ins()
                            .call(list_func, &[count_val, array_ptr, stride_val]);
                    let result = builder.inst_results(call)[0];
                    values.push(result);
                }
                Task::BuildRuntimeSet { expr_id, count } => {
                    let mut element_values: Vec<Value> = (0..count)
                        .map(|_| values.pop().expect("set element value"))
                        .collect();
                    element_values.reverse();

                    let elem_abi = {
                        let layout = kernel.exprs()[expr_id].layout;
                        let LayoutKind::Set { element } =
                            &self.program.layouts()[layout].kind.clone()
                        else {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "BuildRuntimeSet requires Set layout",
                            ));
                        };
                        self.field_abi_shape(kernel_id, *element, "set element")?
                    };
                    let stride = elem_abi.size.max(1);
                    let array_size = (count as u32) * stride;
                    let array_slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            array_size.max(8),
                            elem_abi.align.max(1).ilog2() as u8,
                        ));
                    let array_ptr = builder.ins().stack_addr(self.pointer_type(), array_slot, 0);
                    for (i, elem_val) in element_values.iter().enumerate() {
                        builder.ins().store(
                            MemFlags::new(),
                            *elem_val,
                            array_ptr,
                            (i as u32 * stride) as i32,
                        );
                    }

                    let set_func = self.declare_set_new_func(kernel_id, builder)?;
                    let count_val = builder.ins().iconst(types::I64, count as i64);
                    let stride_val = builder.ins().iconst(types::I64, stride as i64);
                    let call =
                        builder
                            .ins()
                            .call(set_func, &[count_val, array_ptr, stride_val]);
                    let result = builder.inst_results(call)[0];
                    values.push(result);
                }
                Task::BuildRuntimeMap { expr_id, count } => {
                    let mut kv_values: Vec<Value> = (0..count * 2)
                        .map(|_| values.pop().expect("map key/value"))
                        .collect();
                    kv_values.reverse();

                    let (key_abi, val_abi) = {
                        let layout = kernel.exprs()[expr_id].layout;
                        let LayoutKind::Map { key, value } =
                            &self.program.layouts()[layout].kind.clone()
                        else {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "BuildRuntimeMap requires Map layout",
                            ));
                        };
                        (
                            self.field_abi_shape(kernel_id, *key, "map key")?,
                            self.field_abi_shape(kernel_id, *value, "map value")?,
                        )
                    };
                    let key_stride = key_abi.size.max(1);
                    let val_stride = val_abi.size.max(1);
                    let entry_stride = key_stride + val_stride;
                    let array_size = (count as u32) * entry_stride;
                    let align = key_abi.align.max(val_abi.align).max(1);
                    let array_slot =
                        builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            array_size.max(8),
                            align.ilog2() as u8,
                        ));
                    let array_ptr = builder.ins().stack_addr(self.pointer_type(), array_slot, 0);
                    for i in 0..count {
                        let base_offset = (i as u32) * entry_stride;
                        builder.ins().store(
                            MemFlags::new(),
                            kv_values[i * 2],
                            array_ptr,
                            base_offset as i32,
                        );
                        builder.ins().store(
                            MemFlags::new(),
                            kv_values[i * 2 + 1],
                            array_ptr,
                            (base_offset + key_stride) as i32,
                        );
                    }

                    let map_func = self.declare_map_new_func(kernel_id, builder)?;
                    let count_val = builder.ins().iconst(types::I64, count as i64);
                    let key_size_val = builder.ins().iconst(types::I64, key_stride as i64);
                    let val_size_val = builder.ins().iconst(types::I64, val_stride as i64);
                    let call = builder.ins().call(
                        map_func,
                        &[count_val, array_ptr, key_size_val, val_size_val],
                    );
                    let result = builder.inst_results(call)[0];
                    values.push(result);
                }
                Task::ContinuePipeTruthyFalsy {
                    pipe_expr,
                    stage_index,
                    current,
                    merge_block,
                    falsy_block,
                } => {
                    let truthy_result = values.pop().expect("truthy branch result");
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(truthy_result)]);

                    builder.switch_to_block(falsy_block);

                    let (falsy_payload_subject, falsy_constructor, falsy_body) = {
                        let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                        let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                            unreachable!()
                        };
                        let stage = &pipe.stages[stage_index];
                        let crate::InlinePipeStageKind::TruthyFalsy { falsy, .. } = &stage.kind
                        else {
                            unreachable!()
                        };
                        (falsy.payload_subject, falsy.constructor.clone(), falsy.body)
                    };

                    if let Some(slot) = falsy_payload_subject {
                        let input_layout = {
                            let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                            let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                                unreachable!()
                            };
                            pipe.stages[stage_index].input_layout
                        };
                        let payload = self.extract_truthy_falsy_payload(
                            current,
                            input_layout,
                            &falsy_constructor,
                            builder,
                        );
                        inline_subjects[slot.index()] = Some(payload);
                    }

                    tasks.push(Task::FinalizePipeTruthyFalsy {
                        pipe_expr,
                        stage_index,
                        merge_block,
                    });
                    tasks.push(Task::Visit(falsy_body));
                }
                Task::FinalizePipeTruthyFalsy {
                    pipe_expr,
                    stage_index,
                    merge_block,
                } => {
                    let falsy_result = values.pop().expect("falsy branch result");
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(falsy_result)]);

                    builder.switch_to_block(merge_block);
                    let result = builder.block_params(merge_block)[0];

                    let (result_memo, next_stage_count) = {
                        let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                        let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                            unreachable!()
                        };
                        let stage = &pipe.stages[stage_index];
                        (stage.result_memo, pipe.stages.len())
                    };

                    if let Some(slot) = result_memo {
                        inline_subjects[slot.index()] = Some(result);
                    }
                    values.push(result);
                    if stage_index + 1 < next_stage_count {
                        tasks.push(Task::BuildPipeStage {
                            pipe_expr,
                            stage_index: stage_index + 1,
                        });
                    }
                }
                Task::ContinuePipeCaseArmAfterBody {
                    pipe_expr,
                    stage_index,
                    current,
                    arm_index,
                    merge_block,
                    next_block,
                } => {
                    let arm_result = values.pop().expect("case arm body result");
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(arm_result)]);

                    let (arms_len, next_arm_body, next_arm_pattern, result_memo, next_stage_count) = {
                        let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                        let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                            unreachable!()
                        };
                        let stage = &pipe.stages[stage_index];
                        let crate::InlinePipeStageKind::Case { arms } = &stage.kind else {
                            unreachable!()
                        };
                        let arms_len = arms.len();
                        let next_arm = arm_index + 1;
                        let (body, pattern) = if next_arm < arms_len {
                            (arms[next_arm].body, arms[next_arm].pattern.clone())
                        } else {
                            (arms[0].body, arms[0].pattern.clone())
                        };
                        (
                            arms_len,
                            body,
                            pattern,
                            stage.result_memo,
                            pipe.stages.len(),
                        )
                    };

                    let next_arm_index = arm_index + 1;
                    if next_arm_index < arms_len {
                        let next_blk = next_block.expect("next block for non-last arm");
                        builder.switch_to_block(next_blk);

                        let newer_next_block = if next_arm_index + 1 < arms_len {
                            Some(builder.create_block())
                        } else {
                            None
                        };

                        let arm_body_block = builder.create_block();

                        let input_layout = {
                            let pipe_expr_ref = &kernel.exprs()[pipe_expr];
                            let KernelExprKind::Pipe(pipe) = &pipe_expr_ref.kind else {
                                unreachable!()
                            };
                            pipe.stages[stage_index].input_layout
                        };

                        let cond = self.emit_pattern_test(
                            kernel_id,
                            current,
                            &next_arm_pattern,
                            input_layout,
                            &mut inline_subjects,
                            builder,
                        )?;
                        // When there is no newer next arm, branch to a trap block (exhaustive match)
                        let false_target =
                            newer_next_block.unwrap_or_else(|| builder.create_block());
                        let is_last = newer_next_block.is_none();
                        builder
                            .ins()
                            .brif(cond, arm_body_block, &[], false_target, &[]);
                        if is_last {
                            builder.switch_to_block(false_target);
                            builder
                                .ins()
                                .trap(cranelift_codegen::ir::TrapCode::STACK_OVERFLOW);
                        }
                        builder.switch_to_block(arm_body_block);
                        self.apply_pattern_bindings(
                            kernel_id,
                            current,
                            &next_arm_pattern,
                            input_layout,
                            &mut inline_subjects,
                            builder,
                        );
                        tasks.push(Task::ContinuePipeCaseArmAfterBody {
                            pipe_expr,
                            stage_index,
                            current,
                            arm_index: next_arm_index,
                            merge_block,
                            next_block: newer_next_block,
                        });
                        tasks.push(Task::Visit(next_arm_body));
                    } else {
                        builder.switch_to_block(merge_block);
                        let result = builder.block_params(merge_block)[0];
                        if let Some(slot) = result_memo {
                            inline_subjects[slot.index()] = Some(result);
                        }
                        values.push(result);
                        if stage_index + 1 < next_stage_count {
                            tasks.push(Task::BuildPipeStage {
                                pipe_expr,
                                stage_index: stage_index + 1,
                            });
                        }
                    }
                }
            }
        }

        Ok(values
            .pop()
            .expect("kernel expression lowering should leave one root value"))
    }

    fn lower_subject_reference(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        subject: SubjectRef,
        input: Option<Value>,
        inline_subjects: &[Option<Value>],
    ) -> Result<(Value, LayoutId), CodegenError> {
        match subject {
            SubjectRef::Input => {
                let Some(value) = input else {
                    return Err(CodegenError::MissingInputParameter { kernel: kernel_id });
                };
                let layout = kernel
                    .input_subject
                    .expect("validated backend kernels keep input subjects aligned with codegen");
                Ok((value, layout))
            }
            SubjectRef::Inline(slot) => {
                let layout = *kernel.inline_subjects.get(slot.index()).expect(
                    "validated backend kernels keep inline subject layouts aligned with codegen",
                );
                let Some(Some(value)) = inline_subjects.get(slot.index()).copied() else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "inline subject {slot} has no active value in this Cranelift pipe scope"
                        ),
                    ));
                };
                Ok((value, layout))
            }
        }
    }

    fn plan_item_reference(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        item: ItemId,
    ) -> Result<ItemReferencePlan, CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        let expr_layout = kernel.exprs()[expr_id].layout;
        let item_decl = self
            .program
            .items()
            .get(item)
            .expect("validated backend kernels keep item references aligned with codegen");

        if matches!(item_decl.kind, crate::ItemKind::Signal(_)) {
            if let Some(body) = item_decl.body {
                let body_kernel = self.program.kernels().get(body).expect(
                    "validated backend programs keep signal item body kernels aligned with codegen",
                );
                self.require_layout_match(
                    kernel_id,
                    expr_id,
                    expr_layout,
                    body_kernel.result_layout,
                    &format!("signal item `{}` current-value slot", item_decl.name),
                )?;
            }
            return Ok(ItemReferencePlan::SignalSlot { item });
        }

        let Some(body) = item_decl.body else {
            if item_decl.parameters.is_empty() {
                return Ok(ItemReferencePlan::ImportedSlot { item });
            }
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "item `{}` has no body kernel and still requires an imported callable ABI when referenced without saturation",
                    item_decl.name
                ),
            ));
        };
        let body_kernel = self
            .program
            .kernels()
            .get(body)
            .expect("validated backend programs keep item body kernels aligned with codegen");

        if item_decl.parameters.is_empty() {
            self.require_layout_match(
                kernel_id,
                expr_id,
                expr_layout,
                body_kernel.result_layout,
                &format!("direct item value for `{}`", item_decl.name),
            )?;
            return Ok(ItemReferencePlan::DirectValue { body });
        }

        let (parameters, result_layout) = self.callable_signature(expr_layout);
        if parameters != item_decl.parameters || result_layout != body_kernel.result_layout {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "item `{}` referenced as a callable expects parameter layouts {:?} and result layout{}, found layout{}=`{}`",
                    item_decl.name,
                    item_decl.parameters,
                    body_kernel.result_layout,
                    expr_layout,
                    self.program.layouts()[expr_layout]
                ),
            ));
        }

        Ok(ItemReferencePlan::CallableDescriptor {
            item,
            body,
            arity: item_decl.parameters.len(),
        })
    }

    fn require_compilable_item_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        item: ItemId,
        arguments: &[KernelExprId],
    ) -> Result<DirectApplyPlan, CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        let item_decl = self
            .program
            .items()
            .get(item)
            .expect("validated backend kernels keep item references aligned with codegen");
        // Signal items and items with no body are lowered as external calls.
        if matches!(item_decl.kind, crate::ItemKind::Signal(_)) || item_decl.body.is_none() {
            return Ok(DirectApplyPlan::ExternalItem { item });
        }
        let body = item_decl.body.expect("body checked above");
        if arguments.is_empty() {
            if !item_decl.parameters.is_empty() {
                // Unsaturated reference: emit function address of the body kernel.
                return Ok(DirectApplyPlan::LocalFunctionAddress { body });
            }
        } else if arguments.len() != item_decl.parameters.len() {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "direct item apply to `{}` currently requires saturation: expected {} argument(s), found {}",
                    item_decl.name,
                    item_decl.parameters.len(),
                    arguments.len()
                ),
            ));
        }

        let body_kernel = self
            .program
            .kernels()
            .get(body)
            .expect("validated backend programs keep item body kernels aligned with codegen");
        self.require_layout_match(
            kernel_id,
            expr_id,
            kernel.exprs()[expr_id].layout,
            body_kernel.result_layout,
            &format!("direct item call result for `{}`", item_decl.name),
        )?;
        for (index, (argument, expected_layout)) in arguments
            .iter()
            .zip(item_decl.parameters.iter())
            .enumerate()
        {
            self.require_layout_match(
                kernel_id,
                *argument,
                *expected_layout,
                kernel.exprs()[*argument].layout,
                &format!("direct item call argument {index} for `{}`", item_decl.name),
            )?;
        }
        Ok(DirectApplyPlan::Item { body })
    }

    fn declare_signal_item_slot(&mut self, item: ItemId) -> Result<DataId, CodegenError> {
        if let Some(data_id) = self.declared_signal_slots.get(&item).copied() {
            return Ok(data_id);
        }

        let symbol = signal_slot_symbol(self.program, item);
        let data_id = self
            .module
            .declare_data(&symbol, Linkage::Import, false, false)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: None,
                message: error.to_string().into_boxed_str(),
            })?;
        self.declared_signal_slots.insert(item, data_id);
        Ok(data_id)
    }

    fn lower_signal_item_slot(
        &mut self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        item: ItemId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let item_name = self.program.item_name(item).to_owned();
        let expr_layout = self.program.kernels()[kernel_id].exprs()[expr_id].layout;
        let abi = self.field_abi_shape(
            kernel_id,
            expr_layout,
            &format!("signal item `{item_name}` current-value slot"),
        )?;
        let data_id = self.declare_signal_item_slot(item)?;
        let global = self.module.declare_data_in_func(data_id, builder.func);
        let slot = builder.ins().symbol_value(self.pointer_type(), global);
        Ok(builder.ins().load(abi.ty, MemFlags::new(), slot, 0))
    }

    fn declare_imported_item_slot(&mut self, item: ItemId) -> Result<DataId, CodegenError> {
        if let Some(data_id) = self.declared_imported_item_slots.get(&item).copied() {
            return Ok(data_id);
        }

        let symbol = imported_item_slot_symbol(self.program, item);
        let data_id = self
            .module
            .declare_data(&symbol, Linkage::Import, false, false)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: None,
                message: error.to_string().into_boxed_str(),
            })?;
        self.declared_imported_item_slots.insert(item, data_id);
        Ok(data_id)
    }

    fn lower_imported_item_slot(
        &mut self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        item: ItemId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let item_name = self.program.item_name(item).to_owned();
        let expr_layout = self.program.kernels()[kernel_id].exprs()[expr_id].layout;
        let abi = self.field_abi_shape(
            kernel_id,
            expr_layout,
            &format!("imported item `{item_name}` value slot"),
        )?;
        let data_id = self.declare_imported_item_slot(item)?;
        let global = self.module.declare_data_in_func(data_id, builder.func);
        let slot = builder.ins().symbol_value(self.pointer_type(), global);
        Ok(builder.ins().load(abi.ty, MemFlags::new(), slot, 0))
    }

    fn declare_callable_item_descriptor(
        &mut self,
        item: ItemId,
        body: KernelId,
        arity: usize,
    ) -> Result<DataId, CodegenError> {
        if let Some(data_id) = self.declared_callable_descriptors.get(&item).copied() {
            return Ok(data_id);
        }

        let func_id =
            *self
                .declared_functions
                .get(&body)
                .ok_or_else(|| {
                    CodegenError::CraneliftModule {
                kernel: Some(body),
                message: format!(
                    "item callable descriptor for item{item} requires declared body kernel {body}"
                )
                .into_boxed_str(),
            }
                })?;
        let symbol = callable_descriptor_symbol(self.program, item);
        let data_id = self
            .module
            .declare_data(&symbol, Linkage::Local, false, false)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(body),
                message: error.to_string().into_boxed_str(),
            })?;
        let pointer_bytes = self.pointer_type().bytes() as usize;
        let mut bytes = vec![0; pointer_bytes + 16];
        write_u32_le(&mut bytes, pointer_bytes, item.as_raw());
        write_u32_le(&mut bytes, pointer_bytes + 4, body.as_raw());
        write_u32_le(
            &mut bytes,
            pointer_bytes + 8,
            u32::try_from(arity).map_err(|_| CodegenError::CraneliftModule {
                kernel: Some(body),
                message: format!(
                    "item callable descriptor for item{item} exceeds the current 32-bit arity metadata bound"
                )
                .into_boxed_str(),
            })?,
        );
        write_u32_le(&mut bytes, pointer_bytes + 12, 1);

        let mut data = DataDescription::new();
        data.define(bytes.into_boxed_slice());
        data.set_align(u64::from(self.pointer_type().bytes()).max(8));
        let func = self.module.declare_func_in_data(func_id, &mut data);
        data.write_function_addr(0, func);
        self.module
            .define_data(data_id, &data)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(body),
                message: error.to_string().into_boxed_str(),
            })?;
        self.declared_callable_descriptors.insert(item, data_id);
        Ok(data_id)
    }

    fn lower_item_callable_descriptor(
        &mut self,
        _kernel_id: KernelId,
        item: ItemId,
        body: KernelId,
        arity: usize,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let data_id = self.declare_callable_item_descriptor(item, body, arity)?;
        let global = self.module.declare_data_in_func(data_id, builder.func);
        Ok(builder.ins().symbol_value(self.pointer_type(), global))
    }

    fn resolve_direct_apply_plan(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        callee: KernelExprId,
        arguments: &[KernelExprId],
    ) -> Result<DirectApplyPlan, CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        match &kernel.exprs()[callee].kind {
            KernelExprKind::Item(item) => {
                self.require_compilable_item_call(kernel_id, expr_id, *item, arguments)
            }
            KernelExprKind::SumConstructor(handle) => {
                if arguments.len() != handle.field_count {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "sum constructor `{}.{}` requires exactly {} argument(s), found {}",
                            handle.type_name, handle.variant_name,
                            handle.field_count,
                            arguments.len()
                        ),
                    ));
                }
                let result_layout = kernel.exprs()[expr_id].layout;
                let variant_tag = match &self.program.layouts()[result_layout].kind {
                    LayoutKind::Sum(variants) => variants
                        .iter()
                        .position(|v| v.name.as_ref() == handle.variant_name.as_ref())
                        .ok_or_else(|| self.unsupported_expression(
                            kernel_id, expr_id,
                            &format!("sum constructor variant `{}` not found in layout", handle.variant_name),
                        ))? as i64,
                    LayoutKind::Opaque { .. } | LayoutKind::Domain { .. } => {
                        // Opaque/domain layouts don't carry variant info; use a stable hash
                        sum_variant_tag_for_opaque(handle.variant_name.as_ref())
                    }
                    _ => return Err(self.unsupported_expression(
                        kernel_id, expr_id,
                        "sum constructor apply requires a Sum, Opaque, or Domain result layout",
                    )),
                };
                let payload_layout = if !arguments.is_empty() {
                    Some(kernel.exprs()[arguments[0]].layout)
                } else {
                    None
                };
                Ok(DirectApplyPlan::SumConstruction { variant_tag, payload_layout })
            }
            KernelExprKind::DomainMember(handle) => self
                .require_compilable_domain_member_call(kernel_id, expr_id, callee, handle, arguments)
                .map(DirectApplyPlan::DomainMember),
            KernelExprKind::Builtin(term) => {
                // Ok/Err/Valid/Invalid are sum constructors for Result-like types
                match term {
                    BuiltinTerm::Ok | BuiltinTerm::Err | BuiltinTerm::Valid | BuiltinTerm::Invalid => {
                        let result_layout = kernel.exprs()[expr_id].layout;
                        let (variant_tag, payload_value_layout, payload_error_layout) = match &self.program.layouts()[result_layout].kind {
                            LayoutKind::Result { value, error } => match term {
                                BuiltinTerm::Ok => (0i64, Some(*value), Some(*error)),
                                BuiltinTerm::Err => (1i64, Some(*error), Some(*value)),
                                _ => unreachable!(),
                            },
                            LayoutKind::Validation { value, error } => match term {
                                BuiltinTerm::Valid => (0i64, Some(*value), Some(*error)),
                                BuiltinTerm::Invalid => (1i64, Some(*error), Some(*value)),
                                _ => unreachable!(),
                            },
                            LayoutKind::Sum(variants) => {
                                let variant_name = match term {
                                    BuiltinTerm::Ok => "Ok",
                                    BuiltinTerm::Err => "Err",
                                    BuiltinTerm::Valid => "Valid",
                                    BuiltinTerm::Invalid => "Invalid",
                                    _ => unreachable!(),
                                };
                                let tag = variants.iter().position(|v| v.name.as_ref() == variant_name)
                                    .ok_or_else(|| self.unsupported_expression(
                                        kernel_id, expr_id,
                                        &format!("sum variant `{variant_name}` not found in layout"),
                                    ))? as i64;
                                (tag, None, None)
                            }
                            _ => return Err(self.unsupported_expression(
                                kernel_id, expr_id,
                                &format!("builtin `{term}` apply requires a Result, Validation, or Sum result layout"),
                            )),
                        };
                        let payload_layout = if !arguments.is_empty() {
                            // Use the payload layout from the argument's actual layout
                            Some(kernel.exprs()[arguments[0]].layout)
                        } else {
                            payload_value_layout
                        };
                        let _ = payload_error_layout; // silence unused
                        Ok(DirectApplyPlan::SumConstruction { variant_tag, payload_layout })
                    }
                    _ => self
                        .require_compilable_builtin_call(kernel_id, expr_id, callee, *term, arguments)
                        .map(DirectApplyPlan::Builtin),
                }
            }
            KernelExprKind::IntrinsicValue(intrinsic) => self
                .require_compilable_intrinsic_call(
                    kernel_id,
                    expr_id,
                    callee,
                    *intrinsic,
                    arguments,
                )
                .map(DirectApplyPlan::Intrinsic),
            KernelExprKind::BuiltinClassMember(intrinsic) => Err(
                self.unsupported_builtin_class_member_call(
                    kernel_id,
                    expr_id,
                    *intrinsic,
                    arguments.len(),
                ),
            ),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                "the current Cranelift slice only lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, and niche or inline scalar Option constructors",
            )),
        }
    }

    fn require_compilable_domain_member_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        callee: KernelExprId,
        handle: &aivi_hir::DomainMemberHandle,
        arguments: &[KernelExprId],
    ) -> Result<DomainMemberCallPlan, CodegenError> {
        let detail = format!(
            "domain member `{}.{}`",
            handle.domain_name, handle.member_name
        );
        let (parameters, result_layout) =
            self.require_saturated_callable_call(kernel_id, expr_id, callee, arguments, &detail)?;
        let [parameter_layout] = parameters.as_slice() else {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires backend-owned domain lowering because only unary representational wrappers are compiled in this Cranelift slice"
                ),
            ));
        };

        if domain_member_binary_operator(handle.member_name.as_ref()).is_some()
            || matches!(
                handle.member_name.as_ref(),
                "singleton" | "head" | "tail" | "fromList"
            )
        {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires backend-owned domain/collection lowering beyond representational pointer forwarding"
                ),
            ));
        }

        if self.program.layouts()[*parameter_layout].abi != AbiPassMode::ByReference
            || self.program.layouts()[result_layout].abi != AbiPassMode::ByReference
        {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} currently only lowers representational wrappers when both parameter and result stay by-reference, found layout{parameter_layout}=`{}` -> layout{result_layout}=`{}`",
                    self.program.layouts()[*parameter_layout],
                    self.program.layouts()[result_layout]
                ),
            ));
        }

        match handle.member_name.as_ref() {
            "value" | "unwrap" if self.is_named_domain_layout(*parameter_layout) => {
                Ok(DomainMemberCallPlan::RepresentationalIdentityUnary)
            }
            _ if self.is_named_domain_layout(result_layout) => {
                Ok(DomainMemberCallPlan::RepresentationalIdentityUnary)
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires backend-owned domain lowering beyond representational pointer forwarding"
                ),
            )),
        }
    }

    fn require_compilable_builtin_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        callee: KernelExprId,
        term: BuiltinTerm,
        arguments: &[KernelExprId],
    ) -> Result<BuiltinCallPlan, CodegenError> {
        match term {
            BuiltinTerm::Some => {
                let detail = format!("builtin constructor `{term}`");
                let (_parameters, result_layout) = self.require_saturated_callable_call(
                    kernel_id,
                    expr_id,
                    callee,
                    arguments,
                    &detail,
                )?;
                let [payload] = arguments else {
                    unreachable!("saturated `Some` call should keep exactly one payload");
                };
                let kernel = &self.program.kernels()[kernel_id];
                let contract = self.require_option_codegen_contract(
                    kernel_id,
                    kernel,
                    expr_id,
                    Some(*payload),
                    result_layout,
                    &detail,
                )?;
                Ok(BuiltinCallPlan::OptionSome(contract))
            }
            BuiltinTerm::None => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                "None is not callable; it should appear as a standalone Builtin expression, not as a callee in Apply",
            )),
            // Ok/Err/Valid/Invalid are handled as SumConstruction in
            // resolve_direct_apply_plan before this fallback is reached.
            BuiltinTerm::Ok
            | BuiltinTerm::Err
            | BuiltinTerm::Valid
            | BuiltinTerm::Invalid => unreachable!(
                "builtin constructor `{term}` should be handled as SumConstruction in resolve_direct_apply_plan"
            ),
            BuiltinTerm::True | BuiltinTerm::False => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!("Bool literal `{term}` is not callable"),
            )),
        }
    }

    fn require_compilable_intrinsic_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        callee: KernelExprId,
        intrinsic: IntrinsicValue,
        arguments: &[KernelExprId],
    ) -> Result<IntrinsicCallPlan, CodegenError> {
        let detail = format!("intrinsic `{intrinsic}`");
        let (_parameters, result_layout) =
            self.require_saturated_callable_call(kernel_id, expr_id, callee, arguments, &detail)?;
        let kernel = &self.program.kernels()[kernel_id];
        match intrinsic {
            IntrinsicValue::BytesLength => {
                let [bytes] = arguments else {
                    unreachable!("saturated `BytesLength` call should keep exactly one argument");
                };
                self.require_bytes_expression(
                    kernel_id,
                    *bytes,
                    kernel.exprs()[*bytes].layout,
                    "bytes.length argument",
                )?;
                self.require_int_expression(kernel_id, expr_id, result_layout, "bytes.length result")?;
                Ok(IntrinsicCallPlan::BytesLength)
            }
            IntrinsicValue::BytesGet => {
                let [index, bytes] = arguments else {
                    unreachable!("saturated `BytesGet` call should keep exactly two arguments");
                };
                self.require_int_expression(
                    kernel_id,
                    *index,
                    kernel.exprs()[*index].layout,
                    "bytes.get index",
                )?;
                self.require_bytes_expression(
                    kernel_id,
                    *bytes,
                    kernel.exprs()[*bytes].layout,
                    "bytes.get bytes",
                )?;
                let kind = self.require_inline_scalar_option_expression(
                    kernel_id,
                    kernel,
                    expr_id,
                    None,
                    result_layout,
                    "bytes.get result",
                )?;
                if kind != ScalarOptionKind::Int {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "{detail} currently requires an inline scalar Option Int result, found layout{result_layout}=`{}`",
                            self.program.layouts()[result_layout]
                        ),
                    ));
                }
                Ok(IntrinsicCallPlan::BytesGet)
            }
            IntrinsicValue::BytesFromText => {
                let [text] = arguments else {
                    unreachable!("saturated `BytesFromText` call should keep exactly one argument");
                };
                self.require_text_expression(
                    kernel_id,
                    *text,
                    kernel.exprs()[*text].layout,
                    "bytes.fromText argument",
                )?;
                self.require_bytes_expression(
                    kernel_id,
                    expr_id,
                    result_layout,
                    "bytes.fromText result",
                )?;
                if self.program.layouts()[kernel.exprs()[*text].layout].abi != AbiPassMode::ByReference
                    || self.program.layouts()[result_layout].abi != AbiPassMode::ByReference
                {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "{detail} currently only lowers when both Text input and Bytes result stay by-reference, found layout{}=`{}` -> layout{result_layout}=`{}`",
                            kernel.exprs()[*text].layout,
                            self.program.layouts()[kernel.exprs()[*text].layout],
                            self.program.layouts()[result_layout]
                        ),
                    ));
                }
                Ok(IntrinsicCallPlan::BytesFromText)
            }
            IntrinsicValue::BytesToText => {
                let [bytes] = arguments else {
                    unreachable!("saturated `BytesToText` call should keep exactly one argument");
                };
                self.require_bytes_expression(
                    kernel_id,
                    *bytes,
                    kernel.exprs()[*bytes].layout,
                    "bytes.toText argument",
                )?;
                self.require_niche_option_expression(
                    kernel_id,
                    kernel,
                    expr_id,
                    None,
                    result_layout,
                    "bytes.toText result",
                )?;
                let LayoutKind::Option { element } = &self.program.layouts()[result_layout].kind else {
                    unreachable!("niche option validation should preserve Option layouts");
                };
                self.require_text_expression(
                    kernel_id,
                    expr_id,
                    *element,
                    "bytes.toText result payload",
                )?;
                if self.program.layouts()[result_layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "{detail} currently only lowers when the Option Text result stays by-reference, found layout{result_layout}=`{}`",
                            self.program.layouts()[result_layout]
                        ),
                    ));
                }
                Ok(IntrinsicCallPlan::BytesToText)
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires backend-owned bytes/runtime lowering beyond the current empty/length/get/fromText/toText Cranelift subset"
                ),
            )),
        }
    }

    fn require_saturated_callable_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        callee: KernelExprId,
        arguments: &[KernelExprId],
        detail: &str,
    ) -> Result<(Vec<LayoutId>, LayoutId), CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        let (parameters, result_layout) = self.callable_signature(kernel.exprs()[callee].layout);
        if arguments.len() != parameters.len() {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "direct call to {detail} currently requires saturation: expected {} argument(s), found {}",
                    parameters.len(),
                    arguments.len()
                ),
            ));
        }
        self.require_layout_match(
            kernel_id,
            expr_id,
            kernel.exprs()[expr_id].layout,
            result_layout,
            &format!("direct call result for {detail}"),
        )?;
        for (index, (argument, expected_layout)) in
            arguments.iter().zip(parameters.iter()).enumerate()
        {
            self.require_layout_match(
                kernel_id,
                *argument,
                *expected_layout,
                kernel.exprs()[*argument].layout,
                &format!("direct call argument {index} for {detail}"),
            )?;
        }
        Ok((parameters, result_layout))
    }

    fn callable_signature(&self, layout: LayoutId) -> (Vec<LayoutId>, LayoutId) {
        let mut parameters = Vec::new();
        let mut result = layout;
        loop {
            let Some(layout) = self.program.layouts().get(result) else {
                return (parameters, result);
            };
            let LayoutKind::Arrow {
                parameter,
                result: next_result,
            } = &layout.kind
            else {
                return (parameters, result);
            };
            parameters.push(*parameter);
            result = *next_result;
        }
    }

    fn is_named_domain_layout(&self, layout: LayoutId) -> bool {
        matches!(
            self.program
                .layouts()
                .get(layout)
                .map(|layout| &layout.kind),
            Some(LayoutKind::Domain { .. })
        )
    }

    fn lower_direct_item_call(
        &mut self,
        kernel_id: KernelId,
        body: KernelId,
        arguments: &[Value],
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let func_id = *self.declared_functions.get(&body).ok_or_else(|| {
            self.unsupported_expression(
                kernel_id,
                self.program.kernels()[kernel_id].root,
                &format!("item body kernel {body} was not declared before call lowering"),
            )
        })?;
        let local = self.module.declare_func_in_func(func_id, builder.func);
        let call = builder.ins().call(local, arguments);
        let results = builder.inst_results(call);
        let [result] = results else {
            unreachable!("backend kernels always return exactly one value");
        };
        Ok(*result)
    }

    fn lower_direct_apply(
        &mut self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        plan: DirectApplyPlan,
        arguments: &[Value],
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        match plan {
            DirectApplyPlan::Item { body } => {
                self.lower_direct_item_call(kernel_id, body, arguments, builder)
            }
            DirectApplyPlan::ExternalItem { item } => {
                let result_layout = self.program.kernels()[kernel_id].exprs()[expr_id].layout;
                let arg_types: Vec<cranelift_codegen::ir::Type> = arguments
                    .iter()
                    .map(|&v| builder.func.dfg.value_type(v))
                    .collect();
                let func_id =
                    self.declare_external_item_func(kernel_id, item, &arg_types, result_layout)?;
                let local = self.module.declare_func_in_func(func_id, builder.func);
                let call = builder.ins().call(local, arguments);
                let results = builder.inst_results(call);
                match results {
                    [result] => Ok(*result),
                    _ => Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "external item call returned unexpected number of results",
                    )),
                }
            }
            DirectApplyPlan::LocalFunctionAddress { body } => {
                let func_id = *self.declared_functions.get(&body).ok_or_else(|| {
                    self.unsupported_expression(
                        kernel_id, expr_id,
                        &format!("local function body kernel {} was not declared before address lowering", body),
                    )
                })?;
                let local = self.module.declare_func_in_func(func_id, builder.func);
                Ok(builder.ins().func_addr(self.pointer_type(), local))
            }
            DirectApplyPlan::SumConstruction {
                variant_tag,
                payload_layout,
            } => {
                let tag_size = 8u32;
                let payload_size = if let Some(layout) = payload_layout {
                    self.field_abi_shape(kernel_id, layout, "sum payload")
                        .map(|a| a.size)
                        .unwrap_or(8)
                } else {
                    0u32
                };
                let total_size = (tag_size + payload_size).max(8);
                let slot =
                    builder.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        total_size,
                        3,
                    ));
                let base = builder.ins().stack_addr(self.pointer_type(), slot, 0);
                let tag_val = builder.ins().iconst(types::I64, variant_tag);
                builder.ins().store(MemFlags::new(), tag_val, base, 0);
                if payload_layout.is_some() {
                    if let [payload] = arguments {
                        builder
                            .ins()
                            .store(MemFlags::new(), *payload, base, tag_size as i32);
                    }
                }
                Ok(base)
            }
            DirectApplyPlan::DomainMember(DomainMemberCallPlan::RepresentationalIdentityUnary)
            | DirectApplyPlan::Intrinsic(IntrinsicCallPlan::BytesFromText) => {
                let [argument] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct unary call lowering expected exactly one materialized argument",
                    ));
                };
                Ok(*argument)
            }
            DirectApplyPlan::Builtin(BuiltinCallPlan::OptionSome(contract)) => {
                let [argument] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct Some lowering expected exactly one materialized payload",
                    ));
                };
                match contract {
                    OptionCodegenContract::NicheReference => Ok(*argument),
                    OptionCodegenContract::InlineScalar(kind) => {
                        Ok(self.lower_inline_scalar_option_some(kind, *argument, builder))
                    }
                }
            }
            DirectApplyPlan::Intrinsic(IntrinsicCallPlan::BytesLength) => {
                let [argument] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct bytes.length lowering expected exactly one materialized argument",
                    ));
                };
                Ok(builder
                    .ins()
                    .load(types::I64, MemFlags::new(), *argument, 0))
            }
            DirectApplyPlan::Intrinsic(IntrinsicCallPlan::BytesGet) => {
                let [index, bytes] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct bytes.get lowering expected exactly two materialized arguments",
                    ));
                };
                Ok(self.lower_bytes_get_option(*index, *bytes, builder))
            }
            DirectApplyPlan::Intrinsic(IntrinsicCallPlan::BytesToText) => {
                let [argument] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct bytes.toText lowering expected exactly one materialized argument",
                    ));
                };
                Ok(self.lower_bytes_to_text_option(*argument, builder))
            }
        }
    }

    fn require_layout_match(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        expected: LayoutId,
        found: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        if expected == found {
            return Ok(());
        }
        Err(self.unsupported_expression(
            kernel_id,
            expr_id,
            &format!(
                "{detail} expects layout{expected}=`{}`, found layout{found}=`{}`",
                self.program.layouts()[expected],
                self.program.layouts()[found]
            ),
        ))
    }

    fn unsupported_inline_pipe_stage(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        stage_index: usize,
        detail: &str,
    ) -> CodegenError {
        self.unsupported_expression(
            kernel_id,
            expr_id,
            &format!("inline-pipe stage {stage_index} {detail}"),
        )
    }

    fn materialize_signature_type(
        &self,
        kernel_id: KernelId,
        layout: LayoutId,
        pass_mode: AbiPassMode,
        detail: &str,
    ) -> Result<cranelift_codegen::ir::Type, CodegenError> {
        Ok(self.abi_shape(kernel_id, layout, pass_mode, detail)?.ty)
    }

    fn require_int_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Int) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Int, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_float_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Float) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Float, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_decimal_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Decimal) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Decimal, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_bigint_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::BigInt) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects BigInt, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_arithmetic_expression_triple(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        left: KernelExprId,
        right: KernelExprId,
    ) -> Result<NativeArithmeticKind, CodegenError> {
        let left_layout = &self.program.layouts()[kernel.exprs()[left].layout];
        let right_layout = &self.program.layouts()[kernel.exprs()[right].layout];
        let result_layout = &self.program.layouts()[kernel.exprs()[expr_id].layout];
        let left_layout_id = kernel.exprs()[left].layout;
        let right_layout_id = kernel.exprs()[right].layout;
        let result_layout_id = kernel.exprs()[expr_id].layout;
        match (
            &left_layout.kind,
            &right_layout.kind,
            &result_layout.kind,
        ) {
            (
                LayoutKind::Primitive(PrimitiveType::Int),
                LayoutKind::Primitive(PrimitiveType::Int),
                LayoutKind::Primitive(PrimitiveType::Int),
            ) => Ok(NativeArithmeticKind::Integer),
            (
                LayoutKind::Primitive(PrimitiveType::Decimal),
                LayoutKind::Primitive(PrimitiveType::Decimal),
                LayoutKind::Primitive(PrimitiveType::Decimal),
            ) => Ok(NativeArithmeticKind::Decimal),
            (
                LayoutKind::Primitive(PrimitiveType::BigInt),
                LayoutKind::Primitive(PrimitiveType::BigInt),
                LayoutKind::Primitive(PrimitiveType::BigInt),
            ) => Ok(NativeArithmeticKind::BigInt),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "arithmetic expects matching Int/Decimal/BigInt operands, found \
                     layout{left_layout_id}=`{left_layout}`, \
                     layout{right_layout_id}=`{right_layout}`, \
                     result layout{result_layout_id}=`{result_layout}`"
                ),
            )),
        }
    }

    fn require_bool_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Bool) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Bool, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_bytes_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Bytes) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Bytes, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_text_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Text) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects Text, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn scalar_option_kind_for_layout(&self, layout: LayoutId) -> Option<ScalarOptionKind> {
        match &self.program.layouts()[layout] {
            Layout {
                abi: AbiPassMode::ByValue,
                kind: LayoutKind::Primitive(PrimitiveType::Int),
            } => Some(ScalarOptionKind::Int),
            Layout {
                abi: AbiPassMode::ByValue,
                kind: LayoutKind::Primitive(PrimitiveType::Float),
            } => Some(ScalarOptionKind::Float),
            Layout {
                abi: AbiPassMode::ByValue,
                kind: LayoutKind::Primitive(PrimitiveType::Bool),
            } => Some(ScalarOptionKind::Bool),
            _ => None,
        }
    }

    fn option_codegen_contract(&self, layout: LayoutId) -> Option<OptionCodegenContract> {
        let LayoutKind::Option { element } = &self.program.layouts()[layout].kind else {
            return None;
        };
        if self.program.layouts()[layout].abi == AbiPassMode::ByReference
            && self.program.layouts()[*element].abi == AbiPassMode::ByReference
        {
            return Some(OptionCodegenContract::NicheReference);
        }
        if self.program.layouts()[layout].abi == AbiPassMode::ByValue {
            return self
                .scalar_option_kind_for_layout(*element)
                .map(OptionCodegenContract::InlineScalar);
        }
        None
    }

    fn require_option_codegen_contract(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        payload: Option<KernelExprId>,
        layout: LayoutId,
        detail: &str,
    ) -> Result<OptionCodegenContract, CodegenError> {
        let LayoutKind::Option { element } = &self.program.layouts()[layout].kind else {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} expects an Option layout, found `{}`",
                    self.program.layouts()[layout]
                ),
            ));
        };
        if let Some(payload_expr) = payload {
            let payload_layout = kernel.exprs()[payload_expr].layout;
            if payload_layout != *element {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "{detail} payload expects layout{element}, found layout{payload_layout}=`{}`",
                        self.program.layouts()[payload_layout]
                    ),
                ));
            }
        }
        match self.option_codegen_contract(layout) {
            Some(contract) => Ok(contract),
            None if self.program.layouts()[layout].abi == AbiPassMode::ByReference => {
                Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "{detail} currently requires either a by-reference payload for null-niche lowering or a by-value scalar payload for inline scalar option lowering, found Option layout{layout}=`{}` over payload layout{element}=`{}`",
                        self.program.layouts()[layout],
                        self.program.layouts()[*element]
                    ),
                ))
            }
            None => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires aggregate option encoding beyond the current scalar Option contract, found layout{layout}=`{}` over payload layout{element}=`{}`",
                    self.program.layouts()[layout],
                    self.program.layouts()[*element]
                ),
            )),
        }
    }

    fn require_inline_pipe_gate_contract(
        &self,
        kernel_id: KernelId,
        pipe_expr: KernelExprId,
        stage_index: usize,
        input_layout: LayoutId,
        result_layout: LayoutId,
    ) -> Result<OptionCodegenContract, CodegenError> {
        let LayoutKind::Option { element } = &self.program.layouts()[result_layout].kind else {
            return Err(self.unsupported_inline_pipe_stage(
                kernel_id,
                pipe_expr,
                stage_index,
                &format!(
                    "requires an Option result layout over input layout{input_layout}=`{}`, found layout{result_layout}=`{}`",
                    self.program.layouts()[input_layout],
                    self.program.layouts()[result_layout]
                ),
            ));
        };
        if *element != input_layout {
            return Err(self.unsupported_inline_pipe_stage(
                kernel_id,
                pipe_expr,
                stage_index,
                &format!(
                    "result layout{result_layout}=`{}` must wrap input layout{input_layout}=`{}`, found payload layout{element}=`{}`",
                    self.program.layouts()[result_layout],
                    self.program.layouts()[input_layout],
                    self.program.layouts()[*element]
                ),
            ));
        }
        match self.option_codegen_contract(result_layout) {
            Some(contract) => Ok(contract),
            None if self.program.layouts()[result_layout].abi == AbiPassMode::ByReference => {
                Err(self.unsupported_inline_pipe_stage(
                    kernel_id,
                    pipe_expr,
                    stage_index,
                    &format!(
                        "result currently requires either a by-reference payload for null-niche lowering or a by-value scalar payload for inline scalar option lowering, found Option layout{result_layout}=`{}` over payload layout{element}=`{}`",
                        self.program.layouts()[result_layout],
                        self.program.layouts()[*element]
                    ),
                ))
            }
            None => Err(self.unsupported_inline_pipe_stage(
                kernel_id,
                pipe_expr,
                stage_index,
                &format!(
                    "result still requires aggregate option encoding beyond the current scalar Option contract, found layout{result_layout}=`{}` over payload layout{element}=`{}`",
                    self.program.layouts()[result_layout],
                    self.program.layouts()[*element]
                ),
            )),
        }
    }

    fn require_niche_option_expression(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        payload: Option<KernelExprId>,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
        match self
            .require_option_codegen_contract(kernel_id, kernel, expr_id, payload, layout, detail)?
        {
            OptionCodegenContract::NicheReference => Ok(()),
            OptionCodegenContract::InlineScalar(_) => {
                let LayoutKind::Option { element } = &self.program.layouts()[layout].kind else {
                    unreachable!("validated option contract should preserve Option layouts");
                };
                Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "{detail} currently requires an Option over a by-reference payload so codegen can use a null-pointer niche, found payload layout{element}=`{}`",
                        self.program.layouts()[*element]
                    ),
                ))
            }
        }
    }

    fn require_inline_scalar_option_expression(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        payload: Option<KernelExprId>,
        layout: LayoutId,
        detail: &str,
    ) -> Result<ScalarOptionKind, CodegenError> {
        match self.require_option_codegen_contract(
            kernel_id, kernel, expr_id, payload, layout, detail,
        )? {
            OptionCodegenContract::InlineScalar(kind) => Ok(kind),
            OptionCodegenContract::NicheReference => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} currently requires a by-value scalar Option layout for inline lowering, found `{}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn require_compilable_intrinsic_value(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        intrinsic: IntrinsicValue,
        layout: LayoutId,
    ) -> Result<(), CodegenError> {
        match intrinsic {
            IntrinsicValue::BytesEmpty => {
                self.require_bytes_expression(kernel_id, expr_id, layout, "bytes.empty result")?;
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "bytes.empty currently requires a by-reference Bytes layout, found layout{layout}=`{}`",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                Ok(())
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "intrinsic `{intrinsic}` still requires direct call lowering; only bytes.empty lowers as a first-class intrinsic value in the current Cranelift slice"
                ),
            )),
        }
    }

    fn require_ordered_expression_pair(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        left: KernelExprId,
        right: KernelExprId,
    ) -> Result<NativeCompareKind, CodegenError> {
        let left_layout = self.program.layouts()[kernel.exprs()[left].layout].clone();
        let right_layout = self.program.layouts()[kernel.exprs()[right].layout].clone();
        let left_layout_id = kernel.exprs()[left].layout;
        let right_layout_id = kernel.exprs()[right].layout;
        let kind = match (&left_layout.kind, &right_layout.kind) {
            (
                LayoutKind::Primitive(PrimitiveType::Int),
                LayoutKind::Primitive(PrimitiveType::Int),
            ) => NativeCompareKind::Integer,
            (
                LayoutKind::Primitive(PrimitiveType::Float),
                LayoutKind::Primitive(PrimitiveType::Float),
            ) => NativeCompareKind::Float,
            (
                LayoutKind::Primitive(PrimitiveType::Decimal),
                LayoutKind::Primitive(PrimitiveType::Decimal),
            ) => NativeCompareKind::Decimal,
            (
                LayoutKind::Primitive(PrimitiveType::BigInt),
                LayoutKind::Primitive(PrimitiveType::BigInt),
            ) => NativeCompareKind::BigInt,
            _ => {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "comparison expects matching Int/Float/Decimal/BigInt operands, found layout{left_layout_id}=`{left_layout}` and layout{right_layout_id}=`{right_layout}`"
                    ),
                ));
            }
        };
        self.require_bool_expression(
            kernel_id,
            expr_id,
            kernel.exprs()[expr_id].layout,
            "comparison result",
        )?;
        Ok(kind)
    }

    fn require_equatable_expression_pair(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        left: KernelExprId,
        right: KernelExprId,
    ) -> Result<NativeEqualityShape, CodegenError> {
        let left_layout_id = kernel.exprs()[left].layout;
        let right_layout_id = kernel.exprs()[right].layout;
        let left_layout = self.program.layouts()[left_layout_id].clone();
        let right_layout = self.program.layouts()[right_layout_id].clone();
        if left_layout_id != right_layout_id {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "equality expects matching operand layouts, found layout{left_layout_id}=`{left_layout}` and layout{right_layout_id}=`{right_layout}`"
                ),
            ));
        }
        let mut visited = HashSet::new();
        let kind =
            self.resolve_native_equality_shape(kernel_id, expr_id, left_layout_id, &mut visited)?;
        self.require_bool_expression(
            kernel_id,
            expr_id,
            kernel.exprs()[expr_id].layout,
            "equality result",
        )?;
        Ok(kind)
    }

    fn resolve_native_equality_shape(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        visited: &mut HashSet<LayoutId>,
    ) -> Result<NativeEqualityShape, CodegenError> {
        match &self.program.layouts()[layout].kind {
            LayoutKind::Primitive(PrimitiveType::Int)
            | LayoutKind::Primitive(PrimitiveType::Bool) => Ok(NativeEqualityShape::Integer),
            LayoutKind::Primitive(PrimitiveType::Float) => Ok(NativeEqualityShape::Float),
            LayoutKind::Primitive(PrimitiveType::Decimal) => Ok(NativeEqualityShape::Decimal),
            LayoutKind::Primitive(PrimitiveType::BigInt) => Ok(NativeEqualityShape::BigInt),
            LayoutKind::Primitive(PrimitiveType::Text) => {
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "Text layout{layout}=`{}` must stay by-reference for native equality lowering",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                Ok(NativeEqualityShape::Text)
            }
            LayoutKind::Primitive(PrimitiveType::Bytes) => {
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "Bytes layout{layout}=`{}` must stay by-reference for native equality lowering",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                Ok(NativeEqualityShape::Bytes)
            }
            LayoutKind::Tuple(elements) => {
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "tuple layout{layout}=`{}` must stay by-reference for native equality lowering",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                if !visited.insert(layout) {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "equality for recursive tuple layout{layout}=`{}` still requires a compiled representation bridge",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                let result = (|| {
                    let mut fields = Vec::with_capacity(elements.len());
                    let mut offset = 0u32;
                    for (index, field_layout) in elements.iter().copied().enumerate() {
                        let abi = self.field_abi_shape(
                            kernel_id,
                            field_layout,
                            &format!("tuple field {index} in layout{layout}"),
                        )?;
                        offset = align_to(offset, abi.align);
                        let field_offset =
                            i32::try_from(offset).map_err(|_| CodegenError::UnsupportedLayout {
                                kernel: kernel_id,
                                layout,
                                detail: format!(
                                    "tuple field {index} in layout{layout} would live past the current Cranelift immediate-offset range"
                                )
                                .into_boxed_str(),
                            })?;
                        let shape = self.resolve_native_equality_shape(
                            kernel_id,
                            expr_id,
                            field_layout,
                            visited,
                        )?;
                        fields.push(NativeEqualityField {
                            offset: field_offset,
                            layout: field_layout,
                            shape: Box::new(shape),
                        });
                        offset = offset.checked_add(abi.size).ok_or_else(|| {
                            CodegenError::UnsupportedLayout {
                                kernel: kernel_id,
                                layout,
                                detail: format!(
                                    "tuple layout{layout} overflows backend field-offset computation"
                                )
                                .into_boxed_str(),
                            }
                        })?;
                    }
                    Ok(NativeEqualityShape::Aggregate(fields))
                })();
                visited.remove(&layout);
                result
            }
            LayoutKind::Option { element } => {
                if let Some(kind) = self.scalar_option_kind_for_layout(*element) {
                    if self.program.layouts()[layout].abi != AbiPassMode::ByValue {
                        return Err(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            &format!(
                                "inline scalar Option layout{layout}=`{}` must stay by-value for native equality lowering",
                                self.program.layouts()[layout]
                            ),
                        ));
                    }
                    return Ok(NativeEqualityShape::InlineScalarOption(kind));
                }
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "Option layout{layout}=`{}` must stay by-reference for native equality lowering",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                if self.program.layouts()[*element].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "Option layout{layout}=`{}` still requires aggregate option encoding because payload layout{element}=`{}` is not by-reference",
                            self.program.layouts()[layout],
                            self.program.layouts()[*element]
                        ),
                    ));
                }
                if !visited.insert(layout) {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "equality for recursive Option layout{layout}=`{}` still requires a compiled representation bridge",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                let result = self
                    .resolve_native_equality_shape(kernel_id, expr_id, *element, visited)
                    .map(|payload| NativeEqualityShape::NicheOption {
                        payload: Box::new(payload),
                    });
                visited.remove(&layout);
                result
            }
            LayoutKind::Record(fields) => {
                if self.program.layouts()[layout].abi != AbiPassMode::ByReference {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "record layout{layout}=`{}` must stay by-reference for native equality lowering",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                if !visited.insert(layout) {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        &format!(
                            "equality for recursive record layout{layout}=`{}` still requires a compiled representation bridge",
                            self.program.layouts()[layout]
                        ),
                    ));
                }
                let result = (|| {
                    let mut steps = Vec::with_capacity(fields.len());
                    let mut offset = 0u32;
                    for field in fields {
                        let abi = self.field_abi_shape(
                            kernel_id,
                            field.layout,
                            &format!("record field `{}` in layout{layout}", field.name),
                        )?;
                        offset = align_to(offset, abi.align);
                        let field_offset =
                            i32::try_from(offset).map_err(|_| CodegenError::UnsupportedLayout {
                                kernel: kernel_id,
                                layout,
                                detail: format!(
                                    "record field `{}` in layout{layout} would live past the current Cranelift immediate-offset range",
                                    field.name
                                )
                                .into_boxed_str(),
                            })?;
                        let shape = self.resolve_native_equality_shape(
                            kernel_id,
                            expr_id,
                            field.layout,
                            visited,
                        )?;
                        steps.push(NativeEqualityField {
                            offset: field_offset,
                            layout: field.layout,
                            shape: Box::new(shape),
                        });
                        offset = offset.checked_add(abi.size).ok_or_else(|| {
                            CodegenError::UnsupportedLayout {
                                kernel: kernel_id,
                                layout,
                                detail: format!(
                                    "record layout{layout} overflows backend field-offset computation"
                                )
                                .into_boxed_str(),
                            }
                        })?;
                    }
                    Ok(NativeEqualityShape::Aggregate(steps))
                })();
                visited.remove(&layout);
                result
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "equality for layout{layout}=`{}` still requires a compiled representation bridge beyond native scalar/Text/Bytes/record/tuple/scalar-Option/niche-Option shapes",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn lower_native_equality_shape(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        shape: &NativeEqualityShape,
        lhs: Value,
        rhs: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        match shape {
            NativeEqualityShape::Integer => Ok(builder.ins().icmp(IntCC::Equal, lhs, rhs)),
            NativeEqualityShape::Float => Ok(builder.ins().fcmp(FloatCC::Equal, lhs, rhs)),
            NativeEqualityShape::Decimal | NativeEqualityShape::BigInt => {
                Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    "Decimal/BigInt equality inside aggregate fields requires runtime dispatch; use standalone == instead",
                ))
            }
            NativeEqualityShape::InlineScalarOption(_) => {
                Ok(builder.ins().icmp(IntCC::Equal, lhs, rhs))
            }
            NativeEqualityShape::Text | NativeEqualityShape::Bytes => {
                Ok(self.lower_native_byte_sequence_equality(lhs, rhs, builder))
            }
            NativeEqualityShape::Aggregate(fields) => {
                let mut equal = builder.ins().iconst(types::I8, 1);
                for field in fields {
                    let abi =
                        self.field_abi_shape(kernel_id, field.layout, "native equality field")?;
                    let left_field = builder
                        .ins()
                        .load(abi.ty, MemFlags::new(), lhs, field.offset);
                    let right_field =
                        builder
                            .ins()
                            .load(abi.ty, MemFlags::new(), rhs, field.offset);
                    let field_equal = self.lower_native_equality_shape(
                        kernel_id,
                        expr_id,
                        field.shape.as_ref(),
                        left_field,
                        right_field,
                        builder,
                    )?;
                    equal = builder.ins().band(equal, field_equal);
                }
                Ok(equal)
            }
            NativeEqualityShape::NicheOption { payload } => {
                let zero = builder.ins().iconst(self.pointer_type(), 0);
                let left_is_none = builder.ins().icmp(IntCC::Equal, lhs, zero);
                let right_is_none = builder.ins().icmp(IntCC::Equal, rhs, zero);
                let both_none = builder.ins().band(left_is_none, right_is_none);
                let any_none = builder.ins().bor(left_is_none, right_is_none);

                let payload_block = builder.create_block();
                let merge_block = builder.create_block();
                let bool_ty = builder.func.dfg.value_type(both_none);
                builder.append_block_param(merge_block, bool_ty);
                builder.ins().brif(
                    any_none,
                    merge_block,
                    &[BlockArg::Value(both_none)],
                    payload_block,
                    &[],
                );

                builder.seal_block(payload_block);
                builder.switch_to_block(payload_block);
                let some_equal = self.lower_native_equality_shape(
                    kernel_id,
                    expr_id,
                    payload.as_ref(),
                    lhs,
                    rhs,
                    builder,
                )?;
                builder
                    .ins()
                    .jump(merge_block, &[BlockArg::Value(some_equal)]);

                builder.seal_block(merge_block);
                builder.switch_to_block(merge_block);
                Ok(builder.block_params(merge_block)[0])
            }
        }
    }

    fn lower_native_byte_sequence_equality(
        &self,
        lhs: Value,
        rhs: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let same_pointer = builder.ins().icmp(IntCC::Equal, lhs, rhs);
        let bool_ty = builder.func.dfg.value_type(same_pointer);
        let true_value = builder.ins().iconst(bool_ty, 1);
        let false_value = builder.ins().iconst(bool_ty, 0);
        let load_lengths_block = builder.create_block();
        let loop_block = builder.create_block();
        let compare_block = builder.create_block();
        let done_block = builder.create_block();
        let pointer_ty = self.pointer_type();

        builder.append_block_param(loop_block, types::I64);
        builder.append_block_param(loop_block, types::I64);
        builder.append_block_param(loop_block, pointer_ty);
        builder.append_block_param(loop_block, pointer_ty);
        builder.append_block_param(done_block, bool_ty);

        builder.ins().brif(
            same_pointer,
            done_block,
            &[BlockArg::Value(true_value)],
            load_lengths_block,
            &[],
        );

        builder.seal_block(load_lengths_block);
        builder.switch_to_block(load_lengths_block);
        let left_len = builder.ins().load(types::I64, MemFlags::new(), lhs, 0);
        let right_len = builder.ins().load(types::I64, MemFlags::new(), rhs, 0);
        let same_len = builder.ins().icmp(IntCC::Equal, left_len, right_len);
        let left_bytes = builder.ins().iadd_imm(lhs, 8);
        let right_bytes = builder.ins().iadd_imm(rhs, 8);
        let zero_index = builder.ins().iconst(types::I64, 0);
        builder.ins().brif(
            same_len,
            loop_block,
            &[
                BlockArg::Value(zero_index),
                BlockArg::Value(left_len),
                BlockArg::Value(left_bytes),
                BlockArg::Value(right_bytes),
            ],
            done_block,
            &[BlockArg::Value(false_value)],
        );

        builder.switch_to_block(loop_block);
        let loop_params = builder.block_params(loop_block).to_vec();
        let index = loop_params[0];
        let len = loop_params[1];
        let left_bytes = loop_params[2];
        let right_bytes = loop_params[3];
        let at_end = builder.ins().icmp(IntCC::Equal, index, len);
        builder.ins().brif(
            at_end,
            done_block,
            &[BlockArg::Value(true_value)],
            compare_block,
            &[],
        );

        builder.seal_block(compare_block);
        builder.switch_to_block(compare_block);
        let index_as_ptr = if pointer_ty == types::I64 {
            index
        } else {
            builder.ins().ireduce(pointer_ty, index)
        };
        let left_addr = builder.ins().iadd(left_bytes, index_as_ptr);
        let right_addr = builder.ins().iadd(right_bytes, index_as_ptr);
        let left_byte = builder.ins().load(types::I8, MemFlags::new(), left_addr, 0);
        let right_byte = builder
            .ins()
            .load(types::I8, MemFlags::new(), right_addr, 0);
        let byte_equal = builder.ins().icmp(IntCC::Equal, left_byte, right_byte);
        let next_index = builder.ins().iadd_imm(index, 1);
        builder.ins().brif(
            byte_equal,
            loop_block,
            &[
                BlockArg::Value(next_index),
                BlockArg::Value(len),
                BlockArg::Value(left_bytes),
                BlockArg::Value(right_bytes),
            ],
            done_block,
            &[BlockArg::Value(false_value)],
        );

        builder.seal_block(loop_block);
        builder.seal_block(done_block);
        builder.switch_to_block(done_block);
        builder.block_params(done_block)[0]
    }

    fn lower_inline_scalar_option_none(&self, builder: &mut FunctionBuilder<'_>) -> Value {
        let zero = builder.ins().iconst(types::I64, 0);
        builder.ins().uextend(types::I128, zero)
    }

    fn lower_inline_scalar_option_some(
        &self,
        kind: ScalarOptionKind,
        payload: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let payload_bits = match kind {
            ScalarOptionKind::Int => builder.ins().sextend(types::I128, payload),
            ScalarOptionKind::Float => {
                let bits = builder.ins().bitcast(types::I64, MemFlags::new(), payload);
                builder.ins().uextend(types::I128, bits)
            }
            ScalarOptionKind::Bool => builder.ins().uextend(types::I128, payload),
        };
        let shifted = builder.ins().ishl_imm(payload_bits, 64);
        let tag_i64 = builder.ins().iconst(types::I64, 1);
        let tag = builder.ins().uextend(types::I128, tag_i64);
        builder.ins().bor(shifted, tag)
    }

    fn lower_inline_pipe_gate(
        &self,
        contract: OptionCodegenContract,
        current: Value,
        predicate: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let true_block = builder.create_block();
        let false_block = builder.create_block();
        let done_block = builder.create_block();
        let result_ty = match contract {
            OptionCodegenContract::NicheReference => self.pointer_type(),
            OptionCodegenContract::InlineScalar(_) => types::I128,
        };
        builder.append_block_param(done_block, result_ty);
        builder
            .ins()
            .brif(predicate, true_block, &[], false_block, &[]);

        builder.switch_to_block(true_block);
        let some = match contract {
            OptionCodegenContract::NicheReference => current,
            OptionCodegenContract::InlineScalar(kind) => {
                self.lower_inline_scalar_option_some(kind, current, builder)
            }
        };
        builder.ins().jump(done_block, &[BlockArg::Value(some)]);

        builder.switch_to_block(false_block);
        let none = match contract {
            OptionCodegenContract::NicheReference => builder.ins().iconst(self.pointer_type(), 0),
            OptionCodegenContract::InlineScalar(_) => self.lower_inline_scalar_option_none(builder),
        };
        builder.ins().jump(done_block, &[BlockArg::Value(none)]);

        builder.seal_block(true_block);
        builder.seal_block(false_block);
        builder.seal_block(done_block);
        builder.switch_to_block(done_block);
        builder.block_params(done_block)[0]
    }

    fn lower_bytes_get_option(
        &self,
        index: Value,
        bytes: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let negative_index = builder.ins().icmp_imm(IntCC::SignedLessThan, index, 0);
        let bounds_block = builder.create_block();
        let payload_block = builder.create_block();
        let done_block = builder.create_block();
        let none = self.lower_inline_scalar_option_none(builder);

        builder.append_block_param(bounds_block, types::I64);
        builder.append_block_param(payload_block, types::I64);
        builder.append_block_param(done_block, types::I128);

        builder.ins().brif(
            negative_index,
            done_block,
            &[BlockArg::Value(none)],
            bounds_block,
            &[BlockArg::Value(index)],
        );

        builder.seal_block(bounds_block);
        builder.switch_to_block(bounds_block);
        let index = builder.block_params(bounds_block)[0];
        let len = builder.ins().load(types::I64, MemFlags::new(), bytes, 0);
        let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, len);
        builder.ins().brif(
            in_bounds,
            payload_block,
            &[BlockArg::Value(index)],
            done_block,
            &[BlockArg::Value(none)],
        );

        builder.seal_block(payload_block);
        builder.switch_to_block(payload_block);
        let index = builder.block_params(payload_block)[0];
        let byte =
            self.lower_load_byte_sequence_byte(builder.ins().iadd_imm(bytes, 8), index, builder);
        let some = self.lower_inline_scalar_option_some(ScalarOptionKind::Int, byte, builder);
        builder.ins().jump(done_block, &[BlockArg::Value(some)]);

        builder.seal_block(done_block);
        builder.switch_to_block(done_block);
        builder.block_params(done_block)[0]
    }

    fn lower_bytes_to_text_option(&self, bytes: Value, builder: &mut FunctionBuilder<'_>) -> Value {
        let pointer_ty = self.pointer_type();
        let zero_ptr = builder.ins().iconst(pointer_ty, 0);
        let len = builder.ins().load(types::I64, MemFlags::new(), bytes, 0);
        let bytes_base = builder.ins().iadd_imm(bytes, 8);

        let loop_block = builder.create_block();
        let inspect_block = builder.create_block();
        let non_ascii_block = builder.create_block();
        let non_two_block = builder.create_block();
        let non_three_block = builder.create_block();
        let validate_two_block = builder.create_block();
        let validate_two_body_block = builder.create_block();
        let validate_three_block = builder.create_block();
        let validate_three_body_block = builder.create_block();
        let validate_four_block = builder.create_block();
        let validate_four_body_block = builder.create_block();
        let done_block = builder.create_block();

        builder.append_block_param(loop_block, types::I64);
        builder.append_block_param(inspect_block, types::I64);
        builder.append_block_param(non_ascii_block, types::I64);
        builder.append_block_param(non_ascii_block, types::I64);
        builder.append_block_param(non_two_block, types::I64);
        builder.append_block_param(non_two_block, types::I64);
        builder.append_block_param(non_three_block, types::I64);
        builder.append_block_param(non_three_block, types::I64);
        builder.append_block_param(validate_two_block, types::I64);
        builder.append_block_param(validate_two_body_block, types::I64);
        builder.append_block_param(validate_three_block, types::I64);
        builder.append_block_param(validate_three_block, types::I64);
        builder.append_block_param(validate_three_body_block, types::I64);
        builder.append_block_param(validate_three_body_block, types::I64);
        builder.append_block_param(validate_four_block, types::I64);
        builder.append_block_param(validate_four_block, types::I64);
        builder.append_block_param(validate_four_body_block, types::I64);
        builder.append_block_param(validate_four_body_block, types::I64);
        builder.append_block_param(done_block, pointer_ty);

        let zero_index = builder.ins().iconst(types::I64, 0);
        builder
            .ins()
            .jump(loop_block, &[BlockArg::Value(zero_index)]);

        builder.switch_to_block(loop_block);
        let index = builder.block_params(loop_block)[0];
        let at_end = builder.ins().icmp(IntCC::Equal, index, len);
        builder.ins().brif(
            at_end,
            done_block,
            &[BlockArg::Value(bytes)],
            inspect_block,
            &[BlockArg::Value(index)],
        );

        builder.seal_block(inspect_block);
        builder.switch_to_block(inspect_block);
        let index = builder.block_params(inspect_block)[0];
        let first = self.lower_load_byte_sequence_byte(bytes_base, index, builder);
        let is_ascii = builder.ins().icmp_imm(IntCC::UnsignedLessThan, first, 0x80);
        let next_ascii = builder.ins().iadd_imm(index, 1);
        builder.ins().brif(
            is_ascii,
            loop_block,
            &[BlockArg::Value(next_ascii)],
            non_ascii_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
        );

        builder.seal_block(non_ascii_block);
        builder.switch_to_block(non_ascii_block);
        let non_ascii_params = builder.block_params(non_ascii_block).to_vec();
        let index = non_ascii_params[0];
        let first = non_ascii_params[1];
        let is_two = self.lower_unsigned_byte_range(first, 0xC2, 0xDF, builder);
        builder.ins().brif(
            is_two,
            validate_two_block,
            &[BlockArg::Value(index)],
            non_two_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
        );

        builder.seal_block(validate_two_block);
        builder.seal_block(non_two_block);
        builder.switch_to_block(validate_two_block);
        let index = builder.block_params(validate_two_block)[0];
        let required_end = builder.ins().iadd_imm(index, 2);
        let enough = builder
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, required_end, len);
        builder.ins().brif(
            enough,
            validate_two_body_block,
            &[BlockArg::Value(index)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.seal_block(validate_two_body_block);
        builder.switch_to_block(validate_two_body_block);
        let index = builder.block_params(validate_two_body_block)[0];
        let second = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 1),
            builder,
        );
        let second_ok = self.lower_unsigned_byte_range(second, 0x80, 0xBF, builder);
        let next_index = builder.ins().iadd_imm(index, 2);
        builder.ins().brif(
            second_ok,
            loop_block,
            &[BlockArg::Value(next_index)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.switch_to_block(non_two_block);
        let non_two_params = builder.block_params(non_two_block).to_vec();
        let index = non_two_params[0];
        let first = non_two_params[1];
        let is_three = self.lower_unsigned_byte_range(first, 0xE0, 0xEF, builder);
        builder.ins().brif(
            is_three,
            validate_three_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
            non_three_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
        );

        builder.seal_block(validate_three_block);
        builder.seal_block(non_three_block);
        builder.switch_to_block(validate_three_block);
        let validate_three_params = builder.block_params(validate_three_block).to_vec();
        let index = validate_three_params[0];
        let first = validate_three_params[1];
        let required_end = builder.ins().iadd_imm(index, 3);
        let enough = builder
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, required_end, len);
        builder.ins().brif(
            enough,
            validate_three_body_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.seal_block(validate_three_body_block);
        builder.switch_to_block(validate_three_body_block);
        let validate_three_body_params = builder.block_params(validate_three_body_block).to_vec();
        let index = validate_three_body_params[0];
        let first = validate_three_body_params[1];
        let second = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 1),
            builder,
        );
        let third = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 2),
            builder,
        );
        let head_e0 = builder.ins().icmp_imm(IntCC::Equal, first, 0xE0);
        let head_ed = builder.ins().icmp_imm(IntCC::Equal, first, 0xED);
        let second_default = self.lower_unsigned_byte_range(second, 0x80, 0xBF, builder);
        let second_e0 = self.lower_unsigned_byte_range(second, 0xA0, 0xBF, builder);
        let second_ed = self.lower_unsigned_byte_range(second, 0x80, 0x9F, builder);
        let third_ok = self.lower_unsigned_byte_range(third, 0x80, 0xBF, builder);
        let bool_ty = builder.func.dfg.value_type(head_e0);
        let one = builder.ins().iconst(bool_ty, 1);
        let special = builder.ins().bor(head_e0, head_ed);
        let not_special = builder.ins().bxor(special, one);
        let e0_ok = builder.ins().band(head_e0, second_e0);
        let ed_ok = builder.ins().band(head_ed, second_ed);
        let default_ok = builder.ins().band(not_special, second_default);
        let special_ok = builder.ins().bor(e0_ok, ed_ok);
        let second_ok = builder.ins().bor(special_ok, default_ok);
        let sequence_ok = builder.ins().band(second_ok, third_ok);
        let next_index = builder.ins().iadd_imm(index, 3);
        builder.ins().brif(
            sequence_ok,
            loop_block,
            &[BlockArg::Value(next_index)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.switch_to_block(non_three_block);
        let non_three_params = builder.block_params(non_three_block).to_vec();
        let index = non_three_params[0];
        let first = non_three_params[1];
        let is_four = self.lower_unsigned_byte_range(first, 0xF0, 0xF4, builder);
        builder.ins().brif(
            is_four,
            validate_four_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.seal_block(validate_four_block);
        builder.switch_to_block(validate_four_block);
        let validate_four_params = builder.block_params(validate_four_block).to_vec();
        let index = validate_four_params[0];
        let first = validate_four_params[1];
        let required_end = builder.ins().iadd_imm(index, 4);
        let enough = builder
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, required_end, len);
        builder.ins().brif(
            enough,
            validate_four_body_block,
            &[BlockArg::Value(index), BlockArg::Value(first)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.seal_block(validate_four_body_block);
        builder.switch_to_block(validate_four_body_block);
        let validate_four_body_params = builder.block_params(validate_four_body_block).to_vec();
        let index = validate_four_body_params[0];
        let first = validate_four_body_params[1];
        let second = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 1),
            builder,
        );
        let third = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 2),
            builder,
        );
        let fourth = self.lower_load_byte_sequence_byte(
            bytes_base,
            builder.ins().iadd_imm(index, 3),
            builder,
        );
        let head_f0 = builder.ins().icmp_imm(IntCC::Equal, first, 0xF0);
        let head_f4 = builder.ins().icmp_imm(IntCC::Equal, first, 0xF4);
        let second_default = self.lower_unsigned_byte_range(second, 0x80, 0xBF, builder);
        let second_f0 = self.lower_unsigned_byte_range(second, 0x90, 0xBF, builder);
        let second_f4 = self.lower_unsigned_byte_range(second, 0x80, 0x8F, builder);
        let third_ok = self.lower_unsigned_byte_range(third, 0x80, 0xBF, builder);
        let fourth_ok = self.lower_unsigned_byte_range(fourth, 0x80, 0xBF, builder);
        let bool_ty = builder.func.dfg.value_type(head_f0);
        let one = builder.ins().iconst(bool_ty, 1);
        let special = builder.ins().bor(head_f0, head_f4);
        let not_special = builder.ins().bxor(special, one);
        let f0_ok = builder.ins().band(head_f0, second_f0);
        let f4_ok = builder.ins().band(head_f4, second_f4);
        let default_ok = builder.ins().band(not_special, second_default);
        let special_ok = builder.ins().bor(f0_ok, f4_ok);
        let second_ok = builder.ins().bor(special_ok, default_ok);
        let tail_ok = builder.ins().band(third_ok, fourth_ok);
        let sequence_ok = builder.ins().band(second_ok, tail_ok);
        let next_index = builder.ins().iadd_imm(index, 4);
        builder.ins().brif(
            sequence_ok,
            loop_block,
            &[BlockArg::Value(next_index)],
            done_block,
            &[BlockArg::Value(zero_ptr)],
        );

        builder.seal_block(loop_block);
        builder.seal_block(done_block);
        builder.switch_to_block(done_block);
        builder.block_params(done_block)[0]
    }

    fn lower_byte_sequence_index_address(
        &self,
        bytes_base: Value,
        index: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let pointer_ty = self.pointer_type();
        let index_as_ptr = if pointer_ty == types::I64 {
            index
        } else {
            builder.ins().ireduce(pointer_ty, index)
        };
        builder.ins().iadd(bytes_base, index_as_ptr)
    }

    fn lower_load_byte_sequence_byte(
        &self,
        bytes_base: Value,
        index: Value,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let addr = self.lower_byte_sequence_index_address(bytes_base, index, builder);
        let byte = builder.ins().load(types::I8, MemFlags::new(), addr, 0);
        builder.ins().uextend(types::I64, byte)
    }

    fn lower_unsigned_byte_range(
        &self,
        value: Value,
        start: i64,
        end: i64,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        let at_least = builder
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, value, start);
        let at_most = builder
            .ins()
            .icmp_imm(IntCC::UnsignedLessThanOrEqual, value, end);
        builder.ins().band(at_least, at_most)
    }

    fn lower_projection(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        base: Value,
        base_layout: LayoutId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let steps = self.resolve_projection_steps(kernel_id, kernel, expr_id, base_layout)?;
        let mut current = base;
        for (index, step) in steps.iter().enumerate() {
            let abi = self.field_abi_shape(kernel_id, step.layout, "projected field")?;
            let loaded = builder
                .ins()
                .load(abi.ty, MemFlags::new(), current, step.offset);
            if index + 1 == steps.len() {
                return Ok(loaded);
            }
            if self.program.layouts()[step.layout].abi != AbiPassMode::ByReference {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    "intermediate projection steps must stay by-reference so codegen can keep traversing record storage",
                ));
            }
            current = loaded;
        }

        unreachable!("projection lowering should always return from the final step")
    }

    fn resolve_projection_steps(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        base_layout: LayoutId,
    ) -> Result<Vec<ProjectionStep>, CodegenError> {
        let expr = &kernel.exprs()[expr_id];
        let KernelExprKind::Projection { path, .. } = &expr.kind else {
            unreachable!("projection step resolution requires a projection expression");
        };
        if path.is_empty() {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                "projection paths must contain at least one field",
            ));
        }
        if self.program.layouts()[base_layout].abi != AbiPassMode::ByReference {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "projection base must stay by-reference so codegen can dereference it, found layout{base_layout}=`{}`",
                    self.program.layouts()[base_layout]
                ),
            ));
        }

        let mut current_layout = base_layout;
        let mut steps = Vec::with_capacity(path.len());
        for (index, field) in path.iter().enumerate() {
            let step = self.resolve_record_field(kernel_id, expr_id, current_layout, field)?;
            let is_last = index + 1 == path.len();
            if !is_last && self.program.layouts()[step.layout].abi != AbiPassMode::ByReference {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "projection step `.{field}` resolves to by-value layout{}=`{}` before the path ends",
                        step.layout,
                        self.program.layouts()[step.layout]
                    ),
                ));
            }
            current_layout = step.layout;
            steps.push(step);
        }
        if current_layout != expr.layout {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "projection resolves to layout{current_layout}=`{}`, but the expression promises layout{}=`{}`",
                    self.program.layouts()[current_layout],
                    expr.layout,
                    self.program.layouts()[expr.layout]
                ),
            ));
        }

        Ok(steps)
    }

    fn resolve_record_field(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        record_layout: LayoutId,
        field_name: &str,
    ) -> Result<ProjectionStep, CodegenError> {
        let layout = &self.program.layouts()[record_layout];
        let LayoutKind::Record(fields) = &layout.kind else {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "projection step `.{field_name}` expects a record base, found layout{record_layout}=`{layout}`",
                ),
            ));
        };

        let mut offset = 0u32;
        for field in fields {
            let abi = self.field_abi_shape(
                kernel_id,
                field.layout,
                &format!("record field `{}` in layout{record_layout}", field.name),
            )?;
            offset = align_to(offset, abi.align);
            if field.name.as_ref() == field_name {
                let offset = i32::try_from(offset).map_err(|_| CodegenError::UnsupportedLayout {
                    kernel: kernel_id,
                    layout: record_layout,
                    detail: format!(
                        "record field `{field_name}` in layout{record_layout} would live past the current Cranelift immediate-offset range"
                    )
                    .into_boxed_str(),
                })?;
                return Ok(ProjectionStep {
                    offset,
                    layout: field.layout,
                });
            }
            offset =
                offset
                    .checked_add(abi.size)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedLayout {
                        kernel: kernel_id,
                        layout: record_layout,
                        detail: format!(
                        "record layout{record_layout} overflows backend field-offset computation"
                    )
                        .into_boxed_str(),
                    }
                    })?;
        }

        Err(self.unsupported_expression(
            kernel_id,
            expr_id,
            &format!("record layout{record_layout} has no field `.{field_name}`"),
        ))
    }

    fn field_abi_shape(
        &self,
        kernel_id: KernelId,
        layout: LayoutId,
        detail: &str,
    ) -> Result<AbiShape, CodegenError> {
        self.abi_shape(
            kernel_id,
            layout,
            self.program.layouts()[layout].abi,
            detail,
        )
    }

    fn abi_shape(
        &self,
        kernel_id: KernelId,
        layout: LayoutId,
        pass_mode: AbiPassMode,
        detail: &str,
    ) -> Result<AbiShape, CodegenError> {
        match pass_mode {
            AbiPassMode::ByReference => {
                let ty = self.pointer_type();
                let bytes = u32::from(ty.bytes());
                Ok(AbiShape {
                    ty,
                    size: bytes,
                    align: bytes,
                })
            }
            AbiPassMode::ByValue => match &self.program.layouts()[layout].kind {
                LayoutKind::Primitive(PrimitiveType::Int) => Ok(AbiShape {
                    ty: types::I64,
                    size: 8,
                    align: 8,
                }),
                LayoutKind::Primitive(PrimitiveType::Float) => Ok(AbiShape {
                    ty: types::F64,
                    size: 8,
                    align: 8,
                }),
                LayoutKind::Primitive(PrimitiveType::Bool) => Ok(AbiShape {
                    ty: types::I8,
                    size: 1,
                    align: 1,
                }),
                LayoutKind::Primitive(other) => Err(CodegenError::UnsupportedLayout {
                    kernel: kernel_id,
                    layout,
                    detail: format!(
                        "{detail} uses primitive `{other}`, but the current Cranelift slice only materializes Int, Float, and Bool by value"
                    )
                    .into_boxed_str(),
                }),
                LayoutKind::Option { element }
                    if self.scalar_option_kind_for_layout(*element).is_some() =>
                {
                    Ok(AbiShape {
                        ty: types::I128,
                        size: 16,
                        align: 16,
                    })
                }
                _ => Err(CodegenError::UnsupportedLayout {
                    kernel: kernel_id,
                    layout,
                    detail: format!(
                        "{detail} uses aggregate layout `{}`; aggregate packing stays in backend/codegen and still requires an explicit lowering path",
                        self.program.layouts()[layout]
                    )
                    .into_boxed_str(),
                }),
            },
        }
    }

    fn declare_external_item_func(
        &mut self,
        kernel_id: KernelId,
        item: ItemId,
        arg_types: &[cranelift_codegen::ir::Type],
        result_layout: LayoutId,
    ) -> Result<FuncId, CodegenError> {
        let symbol = {
            let item_decl = self
                .program
                .items()
                .get(item)
                .expect("validated item reference");
            format!(
                "aivi_item_{}",
                sanitize_symbol_component(item_decl.name.as_ref())
            )
        };
        if let Some(&fid) = self.declared_external_funcs.get(symbol.as_str()) {
            return Ok(fid);
        }
        let mut sig = self.module.make_signature();
        for &ty in arg_types {
            sig.params.push(AbiParam::new(ty));
        }
        let result_abi = self.field_abi_shape(kernel_id, result_layout, "external item result")?;
        sig.returns.push(AbiParam::new(result_abi.ty));
        let fid = self
            .module
            .declare_function(&symbol, Linkage::Import, &sig)
            .map_err(|e| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: e.to_string().into_boxed_str(),
            })?;
        self.declared_external_funcs
            .insert(symbol.into_boxed_str(), fid);
        Ok(fid)
    }

    fn declare_text_concat_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_text_concat";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// Declare an imported runtime function with signature `(ptr, ptr) -> ptr`.
    /// Used for Decimal/BigInt binary arithmetic (add, sub, mul, div, mod).
    fn declare_ptr_binop_func(
        &mut self,
        sym: &str,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let ptr = self.pointer_type();
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(ptr));
            sig.params.push(AbiParam::new(ptr));
            sig.returns.push(AbiParam::new(ptr));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// Declare an imported runtime function with signature `(ptr) -> ptr`.
    /// Used for Decimal/BigInt unary negate.
    fn declare_ptr_unop_func(
        &mut self,
        sym: &str,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let ptr = self.pointer_type();
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(ptr));
            sig.returns.push(AbiParam::new(ptr));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// Declare an imported runtime function with signature `(ptr, ptr) -> i8`.
    /// Used for Decimal/BigInt comparison (eq, lt).
    fn declare_ptr_cmp_func(
        &mut self,
        sym: &str,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let ptr = self.pointer_type();
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(ptr));
            sig.params.push(AbiParam::new(ptr));
            sig.returns.push(AbiParam::new(types::I8));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_list_new(count: i64, elements_ptr: ptr, element_size: i64) -> ptr`
    fn declare_list_new_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_list_new";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_set_new(count: i64, elements_ptr: ptr, element_size: i64) -> ptr`
    fn declare_set_new_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_set_new";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_map_new(count: i64, entries_ptr: ptr, key_size: i64, value_size: i64) -> ptr`
    fn declare_map_new_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_map_new";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_list_len(list_ptr: ptr) -> i64`
    fn declare_list_len_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_list_len";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.returns.push(AbiParam::new(types::I64));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_list_get(list_ptr: ptr, index: i64) -> ptr`
    fn declare_list_get_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_list_get";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    /// `aivi_list_slice(list_ptr: ptr, start: i64, element_size: i64) -> ptr`
    fn declare_list_slice_func(
        &mut self,
        kernel_id: KernelId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<cranelift_codegen::ir::FuncRef, CodegenError> {
        let sym = "aivi_list_slice";
        let func_id = if let Some(&fid) = self.declared_external_funcs.get(sym) {
            fid
        } else {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(self.pointer_type()));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(self.pointer_type()));
            let fid = self
                .module
                .declare_function(sym, Linkage::Import, &sig)
                .map_err(|e| CodegenError::CraneliftModule {
                    kernel: Some(kernel_id),
                    message: e.to_string().into_boxed_str(),
                })?;
            self.declared_external_funcs
                .insert(sym.to_owned().into_boxed_str(), fid);
            fid
        };
        Ok(self.module.declare_func_in_func(func_id, builder.func))
    }

    fn emit_truthy_falsy_condition(
        &self,
        kernel_id: KernelId,
        pipe_expr: KernelExprId,
        stage_index: usize,
        current: Value,
        input_layout: LayoutId,
        truthy_constructor: &crate::BuiltinTerm,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        match truthy_constructor {
            crate::BuiltinTerm::True => Ok(current),
            crate::BuiltinTerm::Some => match self.option_codegen_contract(input_layout) {
                Some(OptionCodegenContract::NicheReference) => {
                    Ok(builder.ins().icmp_imm(IntCC::NotEqual, current, 0))
                }
                Some(OptionCodegenContract::InlineScalar(_)) => {
                    let low = builder.ins().ireduce(types::I64, current);
                    Ok(builder.ins().icmp_imm(IntCC::NotEqual, low, 0))
                }
                None => Err(self.unsupported_inline_pipe_stage(
                    kernel_id,
                    pipe_expr,
                    stage_index,
                    "TruthyFalsy Some condition requires Option contract",
                )),
            },
            _ => Err(self.unsupported_inline_pipe_stage(
                kernel_id,
                pipe_expr,
                stage_index,
                &format!(
                    "TruthyFalsy condition for constructor {:?} is not yet supported",
                    truthy_constructor
                ),
            )),
        }
    }

    fn extract_truthy_falsy_payload(
        &self,
        current: Value,
        input_layout: LayoutId,
        constructor: &crate::BuiltinTerm,
        builder: &mut FunctionBuilder<'_>,
    ) -> Value {
        match constructor {
            crate::BuiltinTerm::True | crate::BuiltinTerm::False => current,
            crate::BuiltinTerm::Some => match self.option_codegen_contract(input_layout) {
                Some(OptionCodegenContract::NicheReference) => current,
                Some(OptionCodegenContract::InlineScalar(kind)) => {
                    let shifted = builder.ins().ushr_imm(current, 64);
                    let payload_i64 = builder.ins().ireduce(types::I64, shifted);
                    match kind {
                        ScalarOptionKind::Int => payload_i64,
                        ScalarOptionKind::Float => builder.ins().bitcast(
                            cranelift_codegen::ir::types::F64,
                            MemFlags::new(),
                            payload_i64,
                        ),
                        ScalarOptionKind::Bool => builder
                            .ins()
                            .ireduce(cranelift_codegen::ir::types::I8, payload_i64),
                    }
                }
                None => current,
            },
            _ => current,
        }
    }

    fn emit_pattern_test(
        &mut self,
        kernel_id: KernelId,
        current: Value,
        pattern: &crate::InlinePipePattern,
        input_layout: LayoutId,
        inline_subjects: &mut Vec<Option<Value>>,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        match &pattern.kind {
            crate::InlinePipePatternKind::Wildcard => Ok(builder.ins().iconst(types::I8, 1)),
            crate::InlinePipePatternKind::Binding { .. } => Ok(builder.ins().iconst(types::I8, 1)),
            crate::InlinePipePatternKind::Integer(lit) => {
                let n = lit.raw.parse::<i64>().unwrap_or(0);
                Ok(builder.ins().icmp_imm(IntCC::Equal, current, n))
            }
            crate::InlinePipePatternKind::Text(_s) => Ok(builder.ins().iconst(types::I8, 1)),
            crate::InlinePipePatternKind::Constructor {
                constructor,
                arguments,
            } => {
                match constructor {
                    crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::None) => {
                        match self.option_codegen_contract(input_layout) {
                            Some(OptionCodegenContract::NicheReference) => {
                                Ok(builder.ins().icmp_imm(IntCC::Equal, current, 0))
                            }
                            Some(OptionCodegenContract::InlineScalar(_)) => {
                                let low = builder.ins().ireduce(types::I64, current);
                                Ok(builder.ins().icmp_imm(IntCC::Equal, low, 0))
                            }
                            None => Ok(builder.ins().iconst(types::I8, 1)),
                        }
                    }
                    crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Some) => {
                        match self.option_codegen_contract(input_layout) {
                            Some(OptionCodegenContract::NicheReference) => {
                                let is_some = builder.ins().icmp_imm(IntCC::NotEqual, current, 0);
                                if let [sub_pat] = arguments.as_slice() {
                                    let element_layout =
                                        match &self.program.layouts()[input_layout].kind {
                                            LayoutKind::Option { element } => *element,
                                            _ => input_layout,
                                        };
                                    let payload = current;
                                    let sub_test = self.emit_pattern_test(
                                        kernel_id,
                                        payload,
                                        sub_pat,
                                        element_layout,
                                        inline_subjects,
                                        builder,
                                    )?;
                                    Ok(builder.ins().band(is_some, sub_test))
                                } else {
                                    Ok(is_some)
                                }
                            }
                            Some(OptionCodegenContract::InlineScalar(_)) => {
                                let low = builder.ins().ireduce(types::I64, current);
                                Ok(builder.ins().icmp_imm(IntCC::NotEqual, low, 0))
                            }
                            None => Ok(builder.ins().iconst(types::I8, 1)),
                        }
                    }
                    crate::InlinePipeConstructor::Sum(handle) => {
                        let tag = match &self.program.layouts()[input_layout].kind {
                            LayoutKind::Sum(variants) => variants
                                .iter()
                                .position(|v| v.name.as_ref() == handle.variant_name.as_ref())
                                .unwrap_or(0)
                                as i64,
                            LayoutKind::Opaque { .. } | LayoutKind::Domain { .. } => {
                                sum_variant_tag_for_opaque(handle.variant_name.as_ref())
                            }
                            _ => return Ok(builder.ins().iconst(types::I8, 1)),
                        };
                        let loaded_tag =
                            builder.ins().load(types::I64, MemFlags::new(), current, 0);
                        Ok(builder.ins().icmp_imm(IntCC::Equal, loaded_tag, tag))
                    }
                    // Result/Validation constructors used in patterns
                    crate::InlinePipeConstructor::Builtin(
                        crate::BuiltinTerm::Ok
                        | crate::BuiltinTerm::Err
                        | crate::BuiltinTerm::Valid
                        | crate::BuiltinTerm::Invalid,
                    ) => {
                        let tag = match (constructor, &self.program.layouts()[input_layout].kind) {
                            (
                                crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Ok),
                                LayoutKind::Result { .. },
                            ) => 0i64,
                            (
                                crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Err),
                                LayoutKind::Result { .. },
                            ) => 1i64,
                            (
                                crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Valid),
                                LayoutKind::Validation { .. },
                            ) => 0i64,
                            (
                                crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Invalid),
                                LayoutKind::Validation { .. },
                            ) => 1i64,
                            _ => return Ok(builder.ins().iconst(types::I8, 1)),
                        };
                        let loaded_tag =
                            builder.ins().load(types::I64, MemFlags::new(), current, 0);
                        Ok(builder.ins().icmp_imm(IntCC::Equal, loaded_tag, tag))
                    }
                    _ => Ok(builder.ins().iconst(types::I8, 1)),
                }
            }
            crate::InlinePipePatternKind::Tuple(sub_patterns) => {
                let element_layouts: Vec<LayoutId> =
                    match &self.program.layouts()[input_layout].kind.clone() {
                        LayoutKind::Tuple(elements) => elements.clone(),
                        _ => return Ok(builder.ins().iconst(types::I8, 1)),
                    };
                let mut test = builder.ins().iconst(types::I8, 1);
                let mut offset = 0u32;
                for (i, sub_pat) in sub_patterns.iter().enumerate() {
                    if i >= element_layouts.len() {
                        break;
                    }
                    let elem_layout = element_layouts[i];
                    let abi = self.field_abi_shape(kernel_id, elem_layout, "tuple element")?;
                    offset = align_to(offset, abi.align);
                    let elem_val =
                        builder
                            .ins()
                            .load(abi.ty, MemFlags::new(), current, offset as i32);
                    let sub_test = self.emit_pattern_test(
                        kernel_id,
                        elem_val,
                        sub_pat,
                        elem_layout,
                        inline_subjects,
                        builder,
                    )?;
                    let sub8 = if builder.func.dfg.value_type(sub_test) == types::I8 {
                        sub_test
                    } else {
                        builder.ins().ireduce(types::I8, sub_test)
                    };
                    test = builder.ins().band(test, sub8);
                    offset += abi.size;
                }
                Ok(test)
            }
            crate::InlinePipePatternKind::Record(field_patterns) => {
                let mut test = builder.ins().iconst(types::I8, 1);
                let record_layout = input_layout;
                for field_pat in field_patterns {
                    let (offset, field_layout) = self.compute_record_field_offset(
                        kernel_id,
                        record_layout,
                        &field_pat.label,
                    )?;
                    let abi =
                        self.field_abi_shape(kernel_id, field_layout, "record pattern field")?;
                    let field_val =
                        builder
                            .ins()
                            .load(abi.ty, MemFlags::new(), current, offset as i32);
                    let sub_test = self.emit_pattern_test(
                        kernel_id,
                        field_val,
                        &field_pat.pattern,
                        field_layout,
                        inline_subjects,
                        builder,
                    )?;
                    let sub8 = if builder.func.dfg.value_type(sub_test) == types::I8 {
                        sub_test
                    } else {
                        builder.ins().ireduce(types::I8, sub_test)
                    };
                    test = builder.ins().band(test, sub8);
                }
                Ok(test)
            }
            crate::InlinePipePatternKind::List { elements, rest } => {
                let element_layout = match &self.program.layouts()[input_layout].kind {
                    LayoutKind::List { element } => *element,
                    _ => return Ok(builder.ins().iconst(types::I8, 1)),
                };
                let list_len_func = self.declare_list_len_func(kernel_id, builder)?;
                let len_call = builder.ins().call(list_len_func, &[current]);
                let len = builder.inst_results(len_call)[0];
                let expected = elements.len() as i64;
                let len_test = if rest.is_some() {
                    builder
                        .ins()
                        .icmp_imm(IntCC::SignedGreaterThanOrEqual, len, expected)
                } else {
                    builder.ins().icmp_imm(IntCC::Equal, len, expected)
                };
                let mut test = if builder.func.dfg.value_type(len_test) == types::I8 {
                    len_test
                } else {
                    builder.ins().ireduce(types::I8, len_test)
                };
                let elem_abi =
                    self.field_abi_shape(kernel_id, element_layout, "list pattern element")?;
                for (i, sub_pat) in elements.iter().enumerate() {
                    let list_get_func = self.declare_list_get_func(kernel_id, builder)?;
                    let idx = builder.ins().iconst(types::I64, i as i64);
                    let get_call = builder.ins().call(list_get_func, &[current, idx]);
                    let elem_ptr = builder.inst_results(get_call)[0];
                    let elem_val =
                        builder
                            .ins()
                            .load(elem_abi.ty, MemFlags::new(), elem_ptr, 0);
                    let sub_test = self.emit_pattern_test(
                        kernel_id,
                        elem_val,
                        sub_pat,
                        element_layout,
                        inline_subjects,
                        builder,
                    )?;
                    let sub8 = if builder.func.dfg.value_type(sub_test) == types::I8 {
                        sub_test
                    } else {
                        builder.ins().ireduce(types::I8, sub_test)
                    };
                    test = builder.ins().band(test, sub8);
                }
                Ok(test)
            }
        }
    }

    fn apply_pattern_bindings(
        &mut self,
        kernel_id: KernelId,
        current: Value,
        pattern: &crate::InlinePipePattern,
        input_layout: LayoutId,
        inline_subjects: &mut Vec<Option<Value>>,
        builder: &mut FunctionBuilder<'_>,
    ) {
        match &pattern.kind {
            crate::InlinePipePatternKind::Binding { subject } => {
                inline_subjects[subject.index()] = Some(current);
            }
            crate::InlinePipePatternKind::Constructor {
                constructor,
                arguments,
            } => {
                match constructor {
                    crate::InlinePipeConstructor::Builtin(crate::BuiltinTerm::Some) => {
                        if let [sub_pat] = arguments.as_slice() {
                            let element_layout = match &self.program.layouts()[input_layout].kind {
                                LayoutKind::Option { element } => *element,
                                _ => input_layout,
                            };
                            let payload = match self.option_codegen_contract(input_layout) {
                                Some(OptionCodegenContract::NicheReference) => current,
                                Some(OptionCodegenContract::InlineScalar(kind)) => {
                                    let shifted = builder.ins().ushr_imm(current, 64);
                                    let payload_i64 = builder.ins().ireduce(types::I64, shifted);
                                    match kind {
                                        ScalarOptionKind::Int => payload_i64,
                                        ScalarOptionKind::Float => builder.ins().bitcast(
                                            cranelift_codegen::ir::types::F64,
                                            MemFlags::new(),
                                            payload_i64,
                                        ),
                                        ScalarOptionKind::Bool => builder
                                            .ins()
                                            .ireduce(cranelift_codegen::ir::types::I8, payload_i64),
                                    }
                                }
                                None => current,
                            };
                            self.apply_pattern_bindings(
                                kernel_id,
                                payload,
                                sub_pat,
                                element_layout,
                                inline_subjects,
                                builder,
                            );
                        }
                    }
                    crate::InlinePipeConstructor::Sum(handle) => {
                        if let [sub_pat] = arguments.as_slice() {
                            let payload_layout = match &self.program.layouts()[input_layout].kind {
                                LayoutKind::Sum(variants) => variants
                                    .iter()
                                    .find(|v| v.name.as_ref() == handle.variant_name.as_ref())
                                    .and_then(|v| v.payload),
                                _ => None,
                            };
                            if let Some(pl) = payload_layout {
                                let payload_abi =
                                    self.field_abi_shape(kernel_id, pl, "sum payload").ok();
                                let payload = if let Some(abi) = payload_abi {
                                    builder.ins().load(abi.ty, MemFlags::new(), current, 8)
                                } else {
                                    builder.ins().load(
                                        self.pointer_type(),
                                        MemFlags::new(),
                                        current,
                                        8,
                                    )
                                };
                                self.apply_pattern_bindings(
                                    kernel_id,
                                    payload,
                                    sub_pat,
                                    pl,
                                    inline_subjects,
                                    builder,
                                );
                            } else {
                                // Opaque/Domain layout: payload is at offset 8 as a pointer
                                let payload = builder.ins().load(
                                    self.pointer_type(),
                                    MemFlags::new(),
                                    current,
                                    8,
                                );
                                self.apply_pattern_bindings(
                                    kernel_id,
                                    payload,
                                    sub_pat,
                                    input_layout,
                                    inline_subjects,
                                    builder,
                                );
                            }
                        }
                    }
                    // Result/Validation constructors with payload bindings
                    crate::InlinePipeConstructor::Builtin(
                        crate::BuiltinTerm::Ok
                        | crate::BuiltinTerm::Err
                        | crate::BuiltinTerm::Valid
                        | crate::BuiltinTerm::Invalid,
                    ) => {
                        if let [sub_pat] = arguments.as_slice() {
                            let payload_layout =
                                match (constructor, &self.program.layouts()[input_layout].kind) {
                                    (
                                        crate::InlinePipeConstructor::Builtin(
                                            crate::BuiltinTerm::Ok,
                                        ),
                                        LayoutKind::Result { value, .. },
                                    ) => Some(*value),
                                    (
                                        crate::InlinePipeConstructor::Builtin(
                                            crate::BuiltinTerm::Err,
                                        ),
                                        LayoutKind::Result { error, .. },
                                    ) => Some(*error),
                                    (
                                        crate::InlinePipeConstructor::Builtin(
                                            crate::BuiltinTerm::Valid,
                                        ),
                                        LayoutKind::Validation { value, .. },
                                    ) => Some(*value),
                                    (
                                        crate::InlinePipeConstructor::Builtin(
                                            crate::BuiltinTerm::Invalid,
                                        ),
                                        LayoutKind::Validation { error, .. },
                                    ) => Some(*error),
                                    _ => None,
                                };
                            let payload = builder.ins().load(
                                self.pointer_type(),
                                MemFlags::new(),
                                current,
                                8,
                            );
                            let pl = payload_layout.unwrap_or(input_layout);
                            self.apply_pattern_bindings(
                                kernel_id,
                                payload,
                                sub_pat,
                                pl,
                                inline_subjects,
                                builder,
                            );
                        }
                    }
                    _ => {}
                }
            }
            crate::InlinePipePatternKind::Tuple(sub_patterns) => {
                let element_layouts: Vec<LayoutId> =
                    match &self.program.layouts()[input_layout].kind.clone() {
                        LayoutKind::Tuple(elements) => elements.clone(),
                        _ => return,
                    };
                let mut offset = 0u32;
                for (i, sub_pat) in sub_patterns.iter().enumerate() {
                    if i >= element_layouts.len() {
                        break;
                    }
                    let elem_layout = element_layouts[i];
                    let abi = match self.field_abi_shape(kernel_id, elem_layout, "tuple binding") {
                        Ok(a) => a,
                        Err(_) => break,
                    };
                    offset = align_to(offset, abi.align);
                    let elem_val =
                        builder
                            .ins()
                            .load(abi.ty, MemFlags::new(), current, offset as i32);
                    self.apply_pattern_bindings(
                        kernel_id,
                        elem_val,
                        sub_pat,
                        elem_layout,
                        inline_subjects,
                        builder,
                    );
                    offset += abi.size;
                }
            }
            crate::InlinePipePatternKind::Record(field_patterns) => {
                let record_layout = input_layout;
                for field_pat in field_patterns {
                    let Ok((offset, field_layout)) = self.compute_record_field_offset(
                        kernel_id,
                        record_layout,
                        &field_pat.label,
                    ) else {
                        continue;
                    };
                    let Ok(abi) = self.field_abi_shape(kernel_id, field_layout, "record binding")
                    else {
                        continue;
                    };
                    let field_val =
                        builder
                            .ins()
                            .load(abi.ty, MemFlags::new(), current, offset as i32);
                    self.apply_pattern_bindings(
                        kernel_id,
                        field_val,
                        &field_pat.pattern,
                        field_layout,
                        inline_subjects,
                        builder,
                    );
                }
            }
            crate::InlinePipePatternKind::List { elements, rest } => {
                let element_layout = match &self.program.layouts()[input_layout].kind {
                    LayoutKind::List { element } => *element,
                    _ => return,
                };
                let elem_abi =
                    match self.field_abi_shape(kernel_id, element_layout, "list binding element") {
                        Ok(a) => a,
                        Err(_) => return,
                    };
                for (i, sub_pat) in elements.iter().enumerate() {
                    let list_get_func =
                        match self.declare_list_get_func(kernel_id, builder) {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                    let idx = builder.ins().iconst(types::I64, i as i64);
                    let get_call = builder.ins().call(list_get_func, &[current, idx]);
                    let elem_ptr = builder.inst_results(get_call)[0];
                    let elem_val =
                        builder
                            .ins()
                            .load(elem_abi.ty, MemFlags::new(), elem_ptr, 0);
                    self.apply_pattern_bindings(
                        kernel_id,
                        elem_val,
                        sub_pat,
                        element_layout,
                        inline_subjects,
                        builder,
                    );
                }
                if let Some(rest_pat) = rest {
                    let list_slice_func =
                        match self.declare_list_slice_func(kernel_id, builder) {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                    let start = builder.ins().iconst(types::I64, elements.len() as i64);
                    let stride = builder.ins().iconst(types::I64, elem_abi.size as i64);
                    let slice_call =
                        builder
                            .ins()
                            .call(list_slice_func, &[current, start, stride]);
                    let rest_list = builder.inst_results(slice_call)[0];
                    self.apply_pattern_bindings(
                        kernel_id,
                        rest_list,
                        rest_pat,
                        input_layout,
                        inline_subjects,
                        builder,
                    );
                }
            }
            crate::InlinePipePatternKind::Wildcard
            | crate::InlinePipePatternKind::Integer(_)
            | crate::InlinePipePatternKind::Text(_) => {}
        }
    }

    fn compute_record_field_offset(
        &self,
        kernel_id: KernelId,
        record_layout: LayoutId,
        field_name: &str,
    ) -> Result<(u32, LayoutId), CodegenError> {
        let LayoutKind::Record(fields) = &self.program.layouts()[record_layout].kind else {
            return Err(CodegenError::UnsupportedLayout {
                kernel: kernel_id,
                layout: record_layout,
                detail: format!(
                    "expected Record layout for field offset computation of `{field_name}`"
                )
                .into_boxed_str(),
            });
        };
        let mut offset = 0u32;
        for field in fields {
            let abi = self.field_abi_shape(
                kernel_id,
                field.layout,
                &format!("record field `{}`", field.name),
            )?;
            offset = align_to(offset, abi.align);
            if field.name.as_ref() == field_name {
                return Ok((offset, field.layout));
            }
            offset += abi.size;
        }
        Err(CodegenError::UnsupportedLayout {
            kernel: kernel_id,
            layout: record_layout,
            detail: format!("record has no field `{field_name}`").into_boxed_str(),
        })
    }

    fn pointer_type(&self) -> Type {
        self.module.isa().pointer_type()
    }

    fn materialize_literal_pointer(
        &mut self,
        kernel_id: KernelId,
        family: &str,
        bytes: Box<[u8]>,
        align: u64,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let symbol = format!(
            "aivi_backend_{family}_{}_{}",
            kernel_id.as_raw(),
            self.next_data_symbol
        );
        self.next_data_symbol += 1;
        let data_id = self
            .module
            .declare_data(&symbol, Linkage::Local, false, false)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: error.to_string().into_boxed_str(),
            })?;
        let mut data = DataDescription::new();
        data.define(bytes);
        data.set_align(align);
        self.module
            .define_data(data_id, &data)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: error.to_string().into_boxed_str(),
            })?;
        let global = self.module.declare_data_in_func(data_id, builder.func);
        Ok(builder.ins().symbol_value(self.pointer_type(), global))
    }

    fn lower_intrinsic_value(
        &mut self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        intrinsic: IntrinsicValue,
        layout: LayoutId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        self.require_compilable_intrinsic_value(kernel_id, expr_id, intrinsic, layout)?;
        match intrinsic {
            IntrinsicValue::BytesEmpty => self.materialize_bytes_constant(kernel_id, &[], builder),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "intrinsic `{intrinsic}` still requires direct call lowering; only bytes.empty lowers as a first-class intrinsic value in the current Cranelift slice"
                ),
            )),
        }
    }

    fn materialize_text_constant(
        &mut self,
        kernel_id: KernelId,
        rendered: &str,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        // Backend-native Text literals use a stable, len-prefixed UTF-8 constant cell.
        // The current Cranelift slice treats the resulting pointer opaquely; richer Text
        // operations still need a shared representation contract at the runtime edge.
        self.materialize_byte_sequence_constant(
            kernel_id,
            "text_literal",
            rendered.as_bytes(),
            builder,
        )
    }

    fn materialize_bytes_constant(
        &mut self,
        kernel_id: KernelId,
        bytes: &[u8],
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        self.materialize_byte_sequence_constant(kernel_id, "bytes_literal", bytes, builder)
    }

    fn materialize_byte_sequence_constant(
        &mut self,
        kernel_id: KernelId,
        family: &str,
        bytes: &[u8],
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let mut encoded = Vec::with_capacity(8 + bytes.len());
        encoded.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        encoded.extend_from_slice(bytes);
        self.materialize_literal_pointer(kernel_id, family, encoded.into_boxed_slice(), 8, builder)
    }

    fn can_materialize_static_expression(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
    ) -> Result<bool, CodegenError> {
        let Some(value) = self.evaluate_static_value(kernel_id, kernel, expr_id)? else {
            return Ok(false);
        };
        Ok(self
            .static_materialization_plan(kernel.exprs()[expr_id].layout, &value)
            .is_some())
    }

    fn materialize_static_expression_if_supported(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Option<Value>, CodegenError> {
        let Some(value) = self.evaluate_static_value(kernel_id, kernel, expr_id)? else {
            return Ok(None);
        };
        let Some(plan) = self.static_materialization_plan(kernel.exprs()[expr_id].layout, &value)
        else {
            return Ok(None);
        };
        Ok(Some(
            self.materialize_static_plan(kernel_id, plan, builder)?,
        ))
    }

    fn static_materialization_plan(
        &self,
        layout: LayoutId,
        value: &RuntimeValue,
    ) -> Option<StaticMaterializationPlan> {
        match (&self.program.layouts()[layout].kind, value) {
            (LayoutKind::Primitive(PrimitiveType::Int), RuntimeValue::Int(value)) => {
                Some(StaticMaterializationPlan::Int(*value))
            }
            (LayoutKind::Primitive(PrimitiveType::Float), RuntimeValue::Float(value)) => {
                Some(StaticMaterializationPlan::Float(*value))
            }
            (LayoutKind::Primitive(PrimitiveType::Bool), RuntimeValue::Bool(value)) => {
                Some(StaticMaterializationPlan::Bool(*value))
            }
            (LayoutKind::Primitive(PrimitiveType::Text), RuntimeValue::Text(value)) => {
                Some(StaticMaterializationPlan::Text(value.clone()))
            }
            (LayoutKind::Primitive(PrimitiveType::Bytes), RuntimeValue::Bytes(value)) => {
                Some(StaticMaterializationPlan::Bytes(value.clone()))
            }
            (LayoutKind::Option { element }, RuntimeValue::OptionNone)
                if self.program.layouts()[layout].abi == AbiPassMode::ByValue =>
            {
                self.scalar_option_kind_for_layout(*element)
                    .map(StaticMaterializationPlan::InlineScalarOptionNone)
            }
            (LayoutKind::Option { element }, RuntimeValue::OptionSome(payload))
                if self.program.layouts()[layout].abi == AbiPassMode::ByValue =>
            {
                let kind = self.scalar_option_kind_for_layout(*element)?;
                self.static_materialization_plan(*element, payload)
                    .map(
                        |payload| StaticMaterializationPlan::InlineScalarOptionSome {
                            kind,
                            payload: Box::new(payload),
                        },
                    )
            }
            (LayoutKind::Option { element }, RuntimeValue::OptionNone)
                if self.supports_static_niche_option_payload(*element) =>
            {
                Some(StaticMaterializationPlan::NicheOptionNone)
            }
            (LayoutKind::Option { element }, RuntimeValue::OptionSome(payload))
                if self.supports_static_niche_option_payload(*element) =>
            {
                self.static_materialization_plan(*element, payload)
                    .map(|payload| StaticMaterializationPlan::NicheOptionSome(Box::new(payload)))
            }
            _ => None,
        }
    }

    fn supports_static_niche_option_payload(&self, layout: LayoutId) -> bool {
        matches!(
            &self.program.layouts()[layout].kind,
            LayoutKind::Primitive(PrimitiveType::Text | PrimitiveType::Bytes)
        ) && self.program.layouts()[layout].abi == AbiPassMode::ByReference
    }

    fn materialize_static_plan(
        &mut self,
        kernel_id: KernelId,
        plan: StaticMaterializationPlan,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        match plan {
            StaticMaterializationPlan::Int(value) => Ok(builder.ins().iconst(types::I64, value)),
            StaticMaterializationPlan::Float(value) => {
                Ok(builder.ins().f64const(Ieee64::with_float(value.to_f64())))
            }
            StaticMaterializationPlan::Bool(value) => {
                Ok(builder.ins().iconst(types::I8, i64::from(value)))
            }
            StaticMaterializationPlan::Text(value) => {
                self.materialize_text_constant(kernel_id, value.as_ref(), builder)
            }
            StaticMaterializationPlan::Bytes(value) => {
                self.materialize_bytes_constant(kernel_id, value.as_ref(), builder)
            }
            StaticMaterializationPlan::NicheOptionNone => {
                Ok(builder.ins().iconst(self.pointer_type(), 0))
            }
            StaticMaterializationPlan::NicheOptionSome(payload) => {
                self.materialize_static_plan(kernel_id, *payload, builder)
            }
            StaticMaterializationPlan::InlineScalarOptionNone(_) => {
                Ok(self.lower_inline_scalar_option_none(builder))
            }
            StaticMaterializationPlan::InlineScalarOptionSome { kind, payload } => {
                let payload = self.materialize_static_plan(kernel_id, *payload, builder)?;
                Ok(self.lower_inline_scalar_option_some(kind, payload, builder))
            }
        }
    }

    fn materialize_static_scalar_aggregate_expression(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let (bytes, align) =
            self.encode_static_scalar_aggregate_constant(kernel_id, kernel, expr_id)?;
        self.materialize_literal_pointer(kernel_id, "aggregate_literal", bytes, align, builder)
    }

    fn encode_static_scalar_aggregate_constant(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
    ) -> Result<(Box<[u8]>, u64), CodegenError> {
        let expr = &kernel.exprs()[expr_id];
        let Some(value) = self.evaluate_static_value(kernel_id, kernel, expr_id)? else {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                "static aggregate literals currently require every field to fold into a backend-owned by-value scalar constant",
            ));
        };
        match (&expr.kind, &self.program.layouts()[expr.layout].kind, value) {
            (KernelExprKind::Tuple(_), LayoutKind::Tuple(elements), RuntimeValue::Tuple(values)) => {
                if elements.len() != values.len() {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "static tuple literal folded into a mismatched runtime field count",
                    ));
                }
                self.encode_static_scalar_aggregate_fields(
                    kernel_id,
                    expr_id,
                    "tuple literal",
                    elements
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(index, layout)| (layout, index.to_string(), &values[index])),
                )
            }
            (
                KernelExprKind::Record(_),
                LayoutKind::Record(layout_fields),
                RuntimeValue::Record(values),
            ) => {
                if layout_fields.len() != values.len() {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "static record literal folded into a mismatched runtime field count",
                    ));
                }
                let mut fields = Vec::with_capacity(layout_fields.len());
                for (layout_field, value_field) in layout_fields.iter().zip(values.iter()) {
                    if layout_field.name != value_field.label {
                        return Err(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            &format!(
                                "static record literal field `{}` folded into mismatched label `{}`",
                                layout_field.name, value_field.label
                            ),
                        ));
                    }
                    fields.push((layout_field.layout, layout_field.name.to_string(), &value_field.value));
                }
                self.encode_static_scalar_aggregate_fields(
                    kernel_id,
                    expr_id,
                    "record literal",
                    fields,
                )
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                "static aggregate literals currently require tuple/record layouts that fold into matching native constants",
            )),
        }
    }

    fn encode_static_scalar_aggregate_fields<'b>(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        detail: &str,
        fields: impl IntoIterator<Item = (LayoutId, String, &'b RuntimeValue)>,
    ) -> Result<(Box<[u8]>, u64), CodegenError> {
        let mut encoded = Vec::new();
        let mut offset = 0u32;
        let mut max_align = 1u32;
        for (layout, label, value) in fields {
            if self.program.layouts()[layout].abi != AbiPassMode::ByValue {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "{detail} field `{label}` still requires a native by-reference constant contract for layout{layout}=`{}`",
                        self.program.layouts()[layout]
                    ),
                ));
            }
            let abi =
                self.field_abi_shape(kernel_id, layout, &format!("{detail} field `{label}`"))?;
            max_align = max_align.max(abi.align);
            offset = align_to(offset, abi.align);
            encoded.resize(offset as usize, 0);
            encoded.extend_from_slice(
                &self.encode_static_scalar_field(
                    kernel_id, expr_id, layout, value, detail, &label,
                )?,
            );
            offset =
                offset
                    .checked_add(abi.size)
                    .ok_or_else(|| CodegenError::UnsupportedLayout {
                        kernel: kernel_id,
                        layout,
                        detail: format!(
                            "{detail} field `{label}` overflows backend constant packing"
                        )
                        .into_boxed_str(),
                    })?;
        }
        Ok((encoded.into_boxed_slice(), u64::from(max_align)))
    }

    fn encode_static_scalar_field(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        layout: LayoutId,
        value: &RuntimeValue,
        detail: &str,
        label: &str,
    ) -> Result<Vec<u8>, CodegenError> {
        match (&self.program.layouts()[layout].kind, value) {
            (LayoutKind::Primitive(PrimitiveType::Int), RuntimeValue::Int(value)) => {
                Ok(value.to_le_bytes().to_vec())
            }
            (LayoutKind::Primitive(PrimitiveType::Float), RuntimeValue::Float(value)) => {
                Ok(value.to_f64().to_bits().to_le_bytes().to_vec())
            }
            (LayoutKind::Primitive(PrimitiveType::Bool), RuntimeValue::Bool(value)) => {
                Ok(vec![u8::from(*value)])
            }
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} field `{label}` expects a static Int/Float/Bool value for layout{layout}=`{}`, found `{value}`",
                    self.program.layouts()[layout]
                ),
            )),
        }
    }

    fn render_static_text_literal(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        text: &crate::TextLiteral,
    ) -> Result<Option<Box<str>>, CodegenError> {
        let mut rendered = String::new();
        for segment in &text.segments {
            match segment {
                crate::TextSegment::Fragment { raw, .. } => rendered.push_str(raw),
                crate::TextSegment::Interpolation { expr, .. } => {
                    let Some(value) = self.evaluate_static_value(kernel_id, kernel, *expr)? else {
                        return Ok(None);
                    };
                    rendered.push_str(&value.to_string());
                }
            }
        }
        let _ = expr_id;
        Ok(Some(rendered.into_boxed_str()))
    }

    fn evaluate_static_value(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        root: KernelExprId,
    ) -> Result<Option<RuntimeValue>, CodegenError> {
        enum Task<'a> {
            Visit(KernelExprId),
            BuildOptionSome,
            BuildBuiltinConstructor {
                constructor: crate::RuntimeConstructor,
            },
            BuildIntrinsicCall {
                intrinsic: IntrinsicValue,
            },
            BuildSumValue {
                handle: aivi_hir::SumConstructorHandle,
            },
            BuildUnary {
                operator: UnaryOperator,
            },
            BuildBinary {
                operator: BinaryOperator,
            },
            BuildText {
                segments: &'a [crate::TextSegment],
            },
            BuildTuple {
                len: usize,
            },
            BuildList {
                len: usize,
            },
            BuildSet {
                len: usize,
            },
            BuildMap {
                len: usize,
            },
            BuildRecord {
                fields: &'a [crate::RecordExprField],
            },
        }

        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr_id) => {
                    match &kernel.exprs()[expr_id].kind {
                        KernelExprKind::OptionSome { payload } => {
                            tasks.push(Task::BuildOptionSome);
                            tasks.push(Task::Visit(*payload));
                        }
                        KernelExprKind::OptionNone => {
                            values.push(RuntimeValue::OptionNone);
                        }
                        KernelExprKind::Builtin(BuiltinTerm::True) => {
                            values.push(RuntimeValue::Bool(true));
                        }
                        KernelExprKind::Builtin(BuiltinTerm::False) => {
                            values.push(RuntimeValue::Bool(false));
                        }
                        KernelExprKind::Builtin(BuiltinTerm::None) => {
                            values.push(RuntimeValue::OptionNone);
                        }
                        KernelExprKind::IntrinsicValue(IntrinsicValue::BytesEmpty) => {
                            values.push(RuntimeValue::Bytes(Box::new([])));
                        }
                        KernelExprKind::SumConstructor(handle) if handle.field_count == 0 => {
                            values.push(RuntimeValue::Sum(crate::RuntimeSumValue {
                                item: handle.item,
                                type_name: handle.type_name.clone(),
                                variant_name: handle.variant_name.clone(),
                                fields: Vec::new(),
                            }));
                        }
                        KernelExprKind::SumConstructor(_) => {
                            return Ok(None);
                        }
                        KernelExprKind::Builtin(_)
                        | KernelExprKind::IntrinsicValue(_)
                        | KernelExprKind::Subject(_)
                        | KernelExprKind::Environment(_)
                        | KernelExprKind::Item(_)
                        | KernelExprKind::DomainMember(_)
                        | KernelExprKind::BuiltinClassMember(_)
                        | KernelExprKind::Projection { .. }
                        | KernelExprKind::Pipe(_) => {
                            return Ok(None);
                        }
                        KernelExprKind::Integer(integer) => {
                            let value = integer.raw.parse::<i64>().map(RuntimeValue::Int).map_err(
                                |_| CodegenError::InvalidIntegerLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: integer.raw.clone(),
                                },
                            )?;
                            values.push(value);
                        }
                        KernelExprKind::Float(float) => {
                            let value = RuntimeFloat::parse_literal(float.raw.as_ref())
                                .map(RuntimeValue::Float)
                                .ok_or_else(|| CodegenError::InvalidFloatLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: float.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::Decimal(decimal) => {
                            let value = RuntimeDecimal::parse_literal(decimal.raw.as_ref())
                                .map(RuntimeValue::Decimal)
                                .ok_or_else(|| CodegenError::InvalidDecimalLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: decimal.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::BigInt(bigint) => {
                            let value = RuntimeBigInt::parse_literal(bigint.raw.as_ref())
                                .map(RuntimeValue::BigInt)
                                .ok_or_else(|| CodegenError::InvalidBigIntLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: bigint.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::SuffixedInteger(integer) => {
                            values.push(RuntimeValue::SuffixedInteger {
                                raw: integer.raw.clone(),
                                suffix: integer.suffix.clone(),
                            });
                        }
                        KernelExprKind::Text(text) => {
                            tasks.push(Task::BuildText {
                                segments: &text.segments,
                            });
                            for segment in text.segments.iter().rev() {
                                if let crate::TextSegment::Interpolation { expr, .. } = segment {
                                    tasks.push(Task::Visit(*expr));
                                }
                            }
                        }
                        KernelExprKind::Tuple(elements) => {
                            tasks.push(Task::BuildTuple {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::List(elements) => {
                            tasks.push(Task::BuildList {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::Map(entries) => {
                            tasks.push(Task::BuildMap { len: entries.len() });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(entry.value));
                                tasks.push(Task::Visit(entry.key));
                            }
                        }
                        KernelExprKind::Set(elements) => {
                            tasks.push(Task::BuildSet {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::Record(fields) => {
                            tasks.push(Task::BuildRecord { fields });
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(field.value));
                            }
                        }
                        KernelExprKind::Apply { callee, arguments } => {
                            match &kernel.exprs()[*callee].kind {
                                KernelExprKind::Builtin(
                                    BuiltinTerm::Some
                                    | BuiltinTerm::Ok
                                    | BuiltinTerm::Err
                                    | BuiltinTerm::Valid
                                    | BuiltinTerm::Invalid,
                                ) => {
                                    if arguments.len() != 1 {
                                        return Ok(None);
                                    }
                                    let constructor = match kernel.exprs()[*callee].kind {
                                        KernelExprKind::Builtin(BuiltinTerm::Some) => {
                                            crate::RuntimeConstructor::Some
                                        }
                                        KernelExprKind::Builtin(BuiltinTerm::Ok) => {
                                            crate::RuntimeConstructor::Ok
                                        }
                                        KernelExprKind::Builtin(BuiltinTerm::Err) => {
                                            crate::RuntimeConstructor::Err
                                        }
                                        KernelExprKind::Builtin(BuiltinTerm::Valid) => {
                                            crate::RuntimeConstructor::Valid
                                        }
                                        KernelExprKind::Builtin(BuiltinTerm::Invalid) => {
                                            crate::RuntimeConstructor::Invalid
                                        }
                                        _ => unreachable!("matched builtin constructor above"),
                                    };
                                    tasks.push(Task::BuildBuiltinConstructor { constructor });
                                    tasks.push(Task::Visit(arguments[0]));
                                }
                                KernelExprKind::SumConstructor(handle) => {
                                    if arguments.len() != handle.field_count as usize {
                                        return Ok(None);
                                    }
                                    tasks.push(Task::BuildSumValue {
                                        handle: handle.clone(),
                                    });
                                    for argument in arguments.iter().rev() {
                                        tasks.push(Task::Visit(*argument));
                                    }
                                }
                                KernelExprKind::IntrinsicValue(intrinsic) => {
                                    let Some(expected_arity) = static_intrinsic_arity(*intrinsic)
                                    else {
                                        return Ok(None);
                                    };
                                    if arguments.len() != expected_arity {
                                        return Ok(None);
                                    }
                                    tasks.push(Task::BuildIntrinsicCall {
                                        intrinsic: *intrinsic,
                                    });
                                    for argument in arguments.iter().rev() {
                                        tasks.push(Task::Visit(*argument));
                                    }
                                }
                                _ => return Ok(None),
                            }
                        }
                        KernelExprKind::Unary { operator, expr } => {
                            tasks.push(Task::BuildUnary {
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(*expr));
                        }
                        KernelExprKind::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            tasks.push(Task::BuildBinary {
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(*right));
                            tasks.push(Task::Visit(*left));
                        }
                    }
                }
                Task::BuildOptionSome => {
                    let payload = values.pop().expect("static option payload should exist");
                    values.push(RuntimeValue::OptionSome(Box::new(payload)));
                }
                Task::BuildBuiltinConstructor { constructor } => {
                    let payload = values
                        .pop()
                        .expect("static constructor payload should exist");
                    let value = match constructor {
                        crate::RuntimeConstructor::Some => {
                            RuntimeValue::OptionSome(Box::new(payload))
                        }
                        crate::RuntimeConstructor::Ok => RuntimeValue::ResultOk(Box::new(payload)),
                        crate::RuntimeConstructor::Err => {
                            RuntimeValue::ResultErr(Box::new(payload))
                        }
                        crate::RuntimeConstructor::Valid => {
                            RuntimeValue::ValidationValid(Box::new(payload))
                        }
                        crate::RuntimeConstructor::Invalid => {
                            RuntimeValue::ValidationInvalid(Box::new(payload))
                        }
                    };
                    values.push(value);
                }
                Task::BuildSumValue { handle } => {
                    let fields = drain_tail(&mut values, handle.field_count as usize);
                    values.push(RuntimeValue::Sum(crate::RuntimeSumValue {
                        item: handle.item,
                        type_name: handle.type_name,
                        variant_name: handle.variant_name,
                        fields,
                    }));
                }
                Task::BuildIntrinsicCall { intrinsic } => {
                    let Some(value) = static_evaluate_intrinsic_call(
                        intrinsic,
                        drain_tail(
                            &mut values,
                            static_intrinsic_arity(intrinsic).expect(
                                "static intrinsic builder should only use supported arities",
                            ),
                        ),
                    ) else {
                        return Ok(None);
                    };
                    values.push(value);
                }
                Task::BuildUnary { operator } => {
                    let operand = static_strip_signal(
                        values.pop().expect("static unary operand should exist"),
                    );
                    let Some(value) = (match (operator, operand) {
                        (UnaryOperator::Not, RuntimeValue::Bool(value)) => {
                            Some(RuntimeValue::Bool(!value))
                        }
                        _ => None,
                    }) else {
                        return Ok(None);
                    };
                    values.push(value);
                }
                Task::BuildBinary { operator } => {
                    let right =
                        static_strip_signal(values.pop().expect("static binary rhs should exist"));
                    let left =
                        static_strip_signal(values.pop().expect("static binary lhs should exist"));
                    let Some(value) = (match (operator, &left, &right) {
                        (
                            BinaryOperator::GreaterThan,
                            RuntimeValue::Int(left),
                            RuntimeValue::Int(right),
                        ) => Some(RuntimeValue::Bool(left > right)),
                        (
                            BinaryOperator::GreaterThan,
                            RuntimeValue::Float(left),
                            RuntimeValue::Float(right),
                        ) => Some(RuntimeValue::Bool(left.to_f64() > right.to_f64())),
                        (
                            BinaryOperator::LessThan,
                            RuntimeValue::Int(left),
                            RuntimeValue::Int(right),
                        ) => Some(RuntimeValue::Bool(left < right)),
                        (
                            BinaryOperator::LessThan,
                            RuntimeValue::Float(left),
                            RuntimeValue::Float(right),
                        ) => Some(RuntimeValue::Bool(left.to_f64() < right.to_f64())),
                        (
                            BinaryOperator::GreaterThanOrEqual,
                            RuntimeValue::Int(left),
                            RuntimeValue::Int(right),
                        ) => Some(RuntimeValue::Bool(left >= right)),
                        (
                            BinaryOperator::GreaterThanOrEqual,
                            RuntimeValue::Float(left),
                            RuntimeValue::Float(right),
                        ) => Some(RuntimeValue::Bool(left.to_f64() >= right.to_f64())),
                        (
                            BinaryOperator::LessThanOrEqual,
                            RuntimeValue::Int(left),
                            RuntimeValue::Int(right),
                        ) => Some(RuntimeValue::Bool(left <= right)),
                        (
                            BinaryOperator::LessThanOrEqual,
                            RuntimeValue::Float(left),
                            RuntimeValue::Float(right),
                        ) => Some(RuntimeValue::Bool(left.to_f64() <= right.to_f64())),
                        (
                            BinaryOperator::And,
                            RuntimeValue::Bool(left),
                            RuntimeValue::Bool(right),
                        ) => Some(RuntimeValue::Bool(*left && *right)),
                        (
                            BinaryOperator::Or,
                            RuntimeValue::Bool(left),
                            RuntimeValue::Bool(right),
                        ) => Some(RuntimeValue::Bool(*left || *right)),
                        (BinaryOperator::Equals, left, right) => {
                            Some(RuntimeValue::Bool(static_structural_eq(left, right)))
                        }
                        (BinaryOperator::NotEquals, left, right) => {
                            Some(RuntimeValue::Bool(!static_structural_eq(left, right)))
                        }
                        _ => None,
                    }) else {
                        return Ok(None);
                    };
                    values.push(value);
                }
                Task::BuildText { segments } => {
                    let interpolation_count = segments
                        .iter()
                        .filter(|segment| {
                            matches!(segment, crate::TextSegment::Interpolation { .. })
                        })
                        .count();
                    let mut interpolation_values =
                        drain_tail(&mut values, interpolation_count).into_iter();
                    let mut rendered = String::new();
                    for segment in segments {
                        match segment {
                            crate::TextSegment::Fragment { raw, .. } => rendered.push_str(raw),
                            crate::TextSegment::Interpolation { .. } => {
                                let value = interpolation_values
                                    .next()
                                    .expect("static text interpolation should align with values");
                                let value = match value {
                                    RuntimeValue::Signal(inner) => *inner,
                                    other => other,
                                };
                                if matches!(value, RuntimeValue::Callable(_)) {
                                    return Ok(None);
                                }
                                rendered.push_str(&value.to_string());
                            }
                        }
                    }
                    values.push(RuntimeValue::Text(rendered.into_boxed_str()));
                }
                Task::BuildTuple { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::Tuple(elements));
                }
                Task::BuildList { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::List(elements));
                }
                Task::BuildSet { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::Set(elements));
                }
                Task::BuildMap { len } => {
                    let entries = drain_tail(&mut values, len * 2)
                        .chunks_exact(2)
                        .map(|pair| RuntimeMapEntry {
                            key: pair[0].clone(),
                            value: pair[1].clone(),
                        })
                        .collect();
                    values.push(RuntimeValue::Map(RuntimeMap::from_entries(entries)));
                }
                Task::BuildRecord { fields } => {
                    let values_tail = drain_tail(&mut values, fields.len());
                    values.push(RuntimeValue::Record(
                        fields
                            .iter()
                            .map(|field| field.label.clone())
                            .zip(values_tail.into_iter())
                            .map(|(label, value)| RuntimeRecordField { label, value })
                            .collect(),
                    ));
                }
            }
        }

        Ok(values.pop())
    }

    fn unsupported_builtin_class_member_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        intrinsic: crate::BuiltinClassMemberIntrinsic,
        argument_count: usize,
    ) -> CodegenError {
        self.unsupported_expression(
            kernel_id,
            expr_id,
            &format!(
                "builtin class member `{intrinsic:?}` still requires builtin aggregate or higher-order callable lowering; found direct call with {argument_count} argument(s)"
            ),
        )
    }

    fn unsupported_expression(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        detail: &str,
    ) -> CodegenError {
        CodegenError::UnsupportedExpression {
            kernel: kernel_id,
            expr: expr_id,
            detail: format!(
                "{detail}; expression is `{}`",
                describe_expr_kind(&self.program.kernels()[kernel_id].exprs()[expr_id].kind)
            )
            .into_boxed_str(),
        }
    }
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("static evaluator should never drain more values than it has produced");
    values.drain(split..).collect()
}

fn static_intrinsic_arity(intrinsic: IntrinsicValue) -> Option<usize> {
    match intrinsic {
        IntrinsicValue::BytesLength
        | IntrinsicValue::BytesFromText
        | IntrinsicValue::BytesToText => Some(1),
        IntrinsicValue::BytesGet | IntrinsicValue::BytesAppend | IntrinsicValue::BytesRepeat => {
            Some(2)
        }
        IntrinsicValue::BytesSlice => Some(3),
        _ => None,
    }
}

fn static_evaluate_intrinsic_call(
    intrinsic: IntrinsicValue,
    arguments: Vec<RuntimeValue>,
) -> Option<RuntimeValue> {
    let arguments: Vec<_> = arguments.into_iter().map(static_strip_signal).collect();
    match (intrinsic, arguments.as_slice()) {
        (IntrinsicValue::BytesLength, [RuntimeValue::Bytes(bytes)]) => {
            Some(RuntimeValue::Int(bytes.len() as i64))
        }
        (IntrinsicValue::BytesGet, [RuntimeValue::Int(index), RuntimeValue::Bytes(bytes)]) => Some(
            usize::try_from(*index)
                .ok()
                .and_then(|index| bytes.get(index))
                .map(|&byte| RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(byte as i64))))
                .unwrap_or(RuntimeValue::OptionNone),
        ),
        (
            IntrinsicValue::BytesSlice,
            [
                RuntimeValue::Int(from),
                RuntimeValue::Int(to),
                RuntimeValue::Bytes(bytes),
            ],
        ) => {
            let start = (*from as usize).min(bytes.len());
            let end = (*to as usize).min(bytes.len());
            let end = end.max(start);
            Some(RuntimeValue::Bytes(bytes[start..end].into()))
        }
        (IntrinsicValue::BytesAppend, [RuntimeValue::Bytes(left), RuntimeValue::Bytes(right)]) => {
            let mut combined = left.to_vec();
            combined.extend_from_slice(right.as_ref());
            Some(RuntimeValue::Bytes(combined.into()))
        }
        (IntrinsicValue::BytesFromText, [RuntimeValue::Text(text)]) => {
            Some(RuntimeValue::Bytes(text.as_bytes().into()))
        }
        (IntrinsicValue::BytesToText, [RuntimeValue::Bytes(bytes)]) => Some(
            std::str::from_utf8(bytes.as_ref())
                .ok()
                .map(|text| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(text.into()))))
                .unwrap_or(RuntimeValue::OptionNone),
        ),
        (IntrinsicValue::BytesRepeat, [RuntimeValue::Int(byte), RuntimeValue::Int(count)]) => {
            let byte = (*byte).clamp(0, 255) as u8;
            let count = (*count).max(0) as usize;
            Some(RuntimeValue::Bytes(vec![byte; count].into()))
        }
        _ => None,
    }
}

fn static_strip_signal(value: RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(inner) => *inner,
        other => other,
    }
}

fn static_structural_eq(left: &RuntimeValue, right: &RuntimeValue) -> bool {
    let left = match left {
        RuntimeValue::Signal(inner) => inner.as_ref(),
        other => other,
    };
    let right = match right {
        RuntimeValue::Signal(inner) => inner.as_ref(),
        other => other,
    };
    match (left, right) {
        (RuntimeValue::Unit, RuntimeValue::Unit) => true,
        (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => left == right,
        (RuntimeValue::Int(left), RuntimeValue::Int(right)) => left == right,
        (RuntimeValue::Float(left), RuntimeValue::Float(right)) => left == right,
        (RuntimeValue::Decimal(left), RuntimeValue::Decimal(right)) => left == right,
        (RuntimeValue::BigInt(left), RuntimeValue::BigInt(right)) => left == right,
        (RuntimeValue::Text(left), RuntimeValue::Text(right)) => left == right,
        (RuntimeValue::Bytes(left), RuntimeValue::Bytes(right)) => left == right,
        (RuntimeValue::Int(left), RuntimeValue::SuffixedInteger { raw, .. })
        | (RuntimeValue::SuffixedInteger { raw, .. }, RuntimeValue::Int(left)) => {
            raw.parse::<i64>().ok() == Some(*left)
        }
        (
            RuntimeValue::SuffixedInteger {
                raw: left_raw,
                suffix: left_suffix,
            },
            RuntimeValue::SuffixedInteger {
                raw: right_raw,
                suffix: right_suffix,
            },
        ) => left_raw == right_raw && left_suffix == right_suffix,
        (RuntimeValue::Tuple(left), RuntimeValue::Tuple(right))
        | (RuntimeValue::List(left), RuntimeValue::List(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| static_structural_eq(left, right))
        }
        (RuntimeValue::Set(left), RuntimeValue::Set(right)) => {
            static_unordered_values_eq(left, right)
        }
        (RuntimeValue::Map(left), RuntimeValue::Map(right)) => static_unordered_map_eq(left, right),
        (RuntimeValue::Record(left), RuntimeValue::Record(right)) => {
            left.len() == right.len()
                && left.iter().zip(right.iter()).all(|(left, right)| {
                    left.label == right.label && static_structural_eq(&left.value, &right.value)
                })
        }
        (RuntimeValue::Sum(left), RuntimeValue::Sum(right)) => {
            left.item == right.item
                && left.variant_name == right.variant_name
                && left.fields.len() == right.fields.len()
                && left
                    .fields
                    .iter()
                    .zip(right.fields.iter())
                    .all(|(left, right)| static_structural_eq(left, right))
        }
        (RuntimeValue::OptionNone, RuntimeValue::OptionNone) => true,
        (RuntimeValue::OptionSome(left), RuntimeValue::OptionSome(right))
        | (RuntimeValue::ResultOk(left), RuntimeValue::ResultOk(right))
        | (RuntimeValue::ResultErr(left), RuntimeValue::ResultErr(right))
        | (RuntimeValue::ValidationValid(left), RuntimeValue::ValidationValid(right))
        | (RuntimeValue::ValidationInvalid(left), RuntimeValue::ValidationInvalid(right))
        | (RuntimeValue::Signal(left), RuntimeValue::Signal(right)) => {
            static_structural_eq(left, right)
        }
        (RuntimeValue::Callable(_), _)
        | (_, RuntimeValue::Callable(_))
        | (RuntimeValue::Task(_), _)
        | (_, RuntimeValue::Task(_))
        | (RuntimeValue::DbTask(_), _)
        | (_, RuntimeValue::DbTask(_)) => false,
        _ => false,
    }
}

fn static_unordered_values_eq(left: &[RuntimeValue], right: &[RuntimeValue]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut matched = vec![false; right.len()];
    'left_values: for left_value in left {
        for (index, right_value) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if static_structural_eq(left_value, right_value) {
                matched[index] = true;
                continue 'left_values;
            }
        }
        return false;
    }
    true
}

fn static_unordered_map_eq(left: &RuntimeMap, right: &RuntimeMap) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut matched = vec![false; right.len()];
    'left_entries: for (left_key, left_value) in left {
        for (index, (right_key, right_value)) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if static_structural_eq(left_key, right_key)
                && static_structural_eq(left_value, right_value)
            {
                matched[index] = true;
                continue 'left_entries;
            }
        }
        return false;
    }
    true
}

fn domain_member_binary_operator(member_name: &str) -> Option<BinaryOperator> {
    match member_name {
        "+" => Some(BinaryOperator::Add),
        "-" => Some(BinaryOperator::Subtract),
        "*" => Some(BinaryOperator::Multiply),
        "/" => Some(BinaryOperator::Divide),
        "%" => Some(BinaryOperator::Modulo),
        ">" => Some(BinaryOperator::GreaterThan),
        "<" => Some(BinaryOperator::LessThan),
        ">=" => Some(BinaryOperator::GreaterThanOrEqual),
        "<=" => Some(BinaryOperator::LessThanOrEqual),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct AbiShape {
    ty: Type,
    size: u32,
    align: u32,
}

#[derive(Clone, Copy)]
struct ProjectionStep {
    offset: i32,
    layout: LayoutId,
}

fn align_to(offset: u32, align: u32) -> u32 {
    debug_assert!(align.is_power_of_two());
    (offset + (align - 1)) & !(align - 1)
}

fn write_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
    let end = offset + 4;
    bytes[offset..end].copy_from_slice(&value.to_le_bytes());
}

fn sum_variant_tag_for_opaque(variant_name: &str) -> i64 {
    variant_name
        .bytes()
        .fold(5381u64, |h, b| h.wrapping_mul(33).wrapping_add(b as u64)) as i64
}

fn kernel_symbol(program: &Program, kernel_id: KernelId, kernel: &Kernel) -> String {
    format!(
        "aivi_{}_kernel{}_{}",
        sanitize_symbol_component(program.item_name(kernel.origin.item)),
        kernel_id.as_raw(),
        match kernel.origin.kind {
            KernelOriginKind::ItemBody { .. } => "item_body".to_owned(),
            KernelOriginKind::GateTrue { stage_index, .. } => format!("gate_true_s{stage_index}"),
            KernelOriginKind::GateFalse { stage_index, .. } => format!("gate_false_s{stage_index}"),
            KernelOriginKind::SignalFilterPredicate { stage_index, .. } => {
                format!("signal_filter_s{stage_index}")
            }
            KernelOriginKind::PreviousSeed { stage_index, .. } => {
                format!("previous_seed_s{stage_index}")
            }
            KernelOriginKind::DiffFunction { stage_index, .. } => {
                format!("diff_function_s{stage_index}")
            }
            KernelOriginKind::DiffSeed { stage_index, .. } => {
                format!("diff_seed_s{stage_index}")
            }
            KernelOriginKind::FanoutMap { stage_index, .. } => {
                format!("fanout_map_s{stage_index}")
            }
            KernelOriginKind::FanoutFilterPredicate { stage_index, .. } => {
                format!("fanout_filter_s{stage_index}")
            }
            KernelOriginKind::FanoutJoin { stage_index, .. } => {
                format!("fanout_join_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStart { stage_index, .. } => {
                format!("recurrence_start_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStep { stage_index, .. } => {
                format!("recurrence_step_s{stage_index}")
            }
            KernelOriginKind::RecurrenceWakeupWitness { .. } => "recurrence_witness".to_owned(),
            KernelOriginKind::RecurrenceSeed { .. } => "recurrence_seed".to_owned(),
            KernelOriginKind::SourceArgument { index, .. } => {
                format!("source_argument_{index}")
            }
            KernelOriginKind::SourceOption { index, .. } => format!("source_option_{index}"),
        }
    )
}

fn signal_slot_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_signal_slot_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn imported_item_slot_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_import_slot_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn callable_descriptor_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_callable_item_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn sanitize_symbol_component(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "item".to_owned()
    } else {
        out
    }
}

fn wrap_one(error: CodegenError) -> CodegenErrors {
    CodegenErrors::new(vec![error])
}
