use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

use cranelift_codegen::{
    ir::{
        AbiParam, BlockArg, InstBuilder, MemFlags, Type, UserFuncName, Value,
        condcodes::{FloatCC, IntCC},
        immediates::Ieee64, types,
    },
    print_errors::pretty_verifier_error,
    settings, verify_function,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use aivi_hir::IntrinsicValue;

use crate::{
    AbiPassMode, BinaryOperator, BuiltinTerm, CallingConventionKind, EnvSlotId, ItemId, Kernel,
    KernelExprId, KernelExprKind, KernelId, KernelOriginKind, LayoutId, LayoutKind, ParameterRole,
    PrimitiveType, Program, RuntimeMap, RuntimeMapEntry, RuntimeRecordField, RuntimeValue,
    SubjectRef, UnaryOperator, ValidationError, describe_expr_kind,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodegenErrors {
    errors: Vec<CodegenError>,
}

impl CodegenErrors {
    pub fn new(errors: Vec<CodegenError>) -> Self {
        Self { errors }
    }

    pub fn errors(&self) -> &[CodegenError] {
        &self.errors
    }

    pub fn into_errors(self) -> Vec<CodegenError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for CodegenErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CodegenErrors {}

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
/// - it uses a backend-local pointer niche for `Option` over by-reference payloads,
/// - it resolves record projection offsets inside backend/codegen,
/// - it emits backend item-body kernels directly,
/// - it lowers saturated direct item calls, representational by-reference domain-member calls,
///   niche `Option` constructor calls already represented in backend IR,
/// - it lowers selected scalar unary/binary operators, including `Float` comparison/equality,
///   plus native equality for `Text`, record/tuple aggregates, and niche `Option` pointers whose
///   leaves are already codegen-supported, and
/// - it explicitly rejects the remaining apply/domain/builtin aggregate/collection/dynamic-text
///   lowering, plus inline-pipe control-flow/debug stages, until those contracts are owned in this
///   layer.
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
    function_builder_ctx: FunctionBuilderContext,
    next_data_symbol: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectApplyPlan {
    Item { body: KernelId },
    DomainMember(DomainMemberCallPlan),
    Builtin(BuiltinCallPlan),
    Intrinsic(IntrinsicCallPlan),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomainMemberCallPlan {
    RepresentationalIdentityUnary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinCallPlan {
    NicheOptionSome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntrinsicCallPlan {
    BytesLength,
    BytesFromText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeCompareKind {
    Integer,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeEqualityShape {
    Integer,
    Float,
    Text,
    Bytes,
    Aggregate(Vec<NativeEqualityField>),
    NicheOption {
        payload: Box<NativeEqualityShape>,
    },
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
}

impl<'a> CraneliftCompiler<'a> {
    fn new(program: &'a Program) -> Result<Self, CodegenError> {
        let isa_builder =
            cranelift_native::builder().map_err(|message| CodegenError::HostIsaUnavailable {
                message: message.to_owned().into_boxed_str(),
            })?;
        let isa = isa_builder
            .finish(settings::Flags::new(settings::builder()))
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
            function_builder_ctx: FunctionBuilderContext::new(),
            next_data_symbol: 0,
        })
    }

    fn prevalidate(&self) -> Result<(), CodegenErrors> {
        let mut errors = Vec::new();

        for (kernel_id, kernel) in self.program.kernels().iter() {
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
                        if let Err(error) = self.require_niche_option_expression(
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
                        if let Err(error) = self.require_niche_option_expression(
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
                        if let Err(error) = self.require_niche_option_expression(
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
                        match self.render_static_text_literal(kernel_id, kernel, expr_id, text) {
                            Ok(Some(_)) => {}
                            Ok(None) => {
                                errors.push(self.unsupported_expression(
                                    kernel_id,
                                    expr_id,
                                    "text interpolation still requires a native text formatting contract beyond static literal folding",
                                ));
                            }
                            Err(error) => {
                                errors.push(error);
                            }
                        }
                    }
                    KernelExprKind::Tuple(_) | KernelExprKind::Record(_) => {
                        if let Err(error) = self.require_static_scalar_aggregate_expression(
                            kernel_id,
                            kernel,
                            expr_id,
                        ) {
                            errors.push(error);
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
                                    errors.push(self.unsupported_inline_pipe_stage(
                                        kernel_id,
                                        expr_id,
                                        stage_index,
                                        "still requires runtime-side debug effects",
                                    ))
                                }
                                crate::InlinePipeStageKind::Gate { .. } => {
                                    errors.push(self.unsupported_inline_pipe_stage(
                                        kernel_id,
                                        expr_id,
                                        stage_index,
                                        "still requires control-flow/Option branching codegen",
                                    ))
                                }
                                crate::InlinePipeStageKind::Case { .. } => {
                                    errors.push(self.unsupported_inline_pipe_stage(
                                        kernel_id,
                                        expr_id,
                                        stage_index,
                                        "still requires pattern-matching codegen",
                                    ))
                                }
                                crate::InlinePipeStageKind::TruthyFalsy { .. } => {
                                    errors.push(self.unsupported_inline_pipe_stage(
                                        kernel_id,
                                        expr_id,
                                        stage_index,
                                        "still requires branch selection codegen",
                                    ))
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
                                if let Err(error) = self.require_int_expression(
                                    kernel_id,
                                    *left,
                                    kernel.exprs()[*left].layout,
                                    "binary lhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_int_expression(
                                    kernel_id,
                                    *right,
                                    kernel.exprs()[*right].layout,
                                    "binary rhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_int_expression(
                                    kernel_id,
                                    expr_id,
                                    expr.layout,
                                    "binary result",
                                ) {
                                    errors.push(error);
                                }
                            }
                            BinaryOperator::GreaterThan
                            | BinaryOperator::LessThan
                            | BinaryOperator::GreaterThanOrEqual
                            | BinaryOperator::LessThanOrEqual => {
                                if let Err(error) = self.require_ordered_expression_pair(
                                    kernel_id,
                                    kernel,
                                    expr_id,
                                    *left,
                                    *right,
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
                        if let Err(error) =
                            self.require_compilable_item_call(kernel_id, expr_id, *item, &[])
                        {
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
                    KernelExprKind::SumConstructor(_)
                    | KernelExprKind::DomainMember(_)
                    | KernelExprKind::BuiltinClassMember(_)
                    | KernelExprKind::Builtin(_)
                    | KernelExprKind::SuffixedInteger(_)
                    | KernelExprKind::List(_)
                    | KernelExprKind::Map(_)
                    | KernelExprKind::Set(_)
                    => {
                        errors.push(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            "the current Cranelift slice lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, niche Option constructors/carriers, record projection, straight-line inline-pipe transform/tap stages, scalar literals, static scalar tuple/record literals, Int/Bool arithmetic, Int/Float/Bool comparison, and native equality over scalar/Text/Bytes/record/tuple/niche-Option shapes only",
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
            BuildPipeStage {
                pipe_expr: KernelExprId,
                stage_index: usize,
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
            RestoreInlineSubjects(Vec<(usize, Option<Value>)>),
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
            }
            saved
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
                        kernel_id,
                        kernel,
                        expr_id,
                        builder,
                    )? {
                        values.push(value);
                        continue;
                    }
                    match &expr.kind {
                        KernelExprKind::Item(item) => {
                            let body =
                                self.require_compilable_item_call(kernel_id, expr_id, *item, &[])?;
                            values.push(self.lower_direct_item_call(
                                kernel_id,
                                body,
                                &[],
                                builder,
                            )?);
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
                            self.require_niche_option_expression(
                                kernel_id,
                                kernel,
                                expr_id,
                                None,
                                expr.layout,
                                "None carrier",
                            )?;
                            values.push(builder.ins().iconst(self.pointer_type(), 0));
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
                            values.push(
                                self.materialize_text_literal(
                                    kernel_id,
                                    kernel,
                                    expr_id,
                                    text,
                                    builder,
                                )?,
                            );
                        }
                        KernelExprKind::Tuple(_) | KernelExprKind::Record(_) => {
                            values.push(self.materialize_static_scalar_aggregate_expression(
                                kernel_id,
                                kernel,
                                expr_id,
                                builder,
                            )?);
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
                            self.require_niche_option_expression(
                                kernel_id,
                                kernel,
                                expr_id,
                                None,
                                expr.layout,
                                "None constructor",
                            )?;
                            values.push(builder.ins().iconst(self.pointer_type(), 0));
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
                        _ => {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "the current Cranelift slice only lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, niche Option constructors/carriers, record projection, scalar subjects/environment slots, straight-line inline-pipe transform/tap stages, scalar literals, static scalar tuple/record literals, Int/Bool arithmetic, Int/Float/Bool comparison, and native equality over scalar/Text/Bytes/record/tuple/niche-Option shapes",
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
                    self.require_niche_option_expression(
                        kernel_id,
                        kernel,
                        expr_id,
                        Some(*payload),
                        expr.layout,
                        "Some carrier",
                    )?;
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
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "add lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "add rhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "add result",
                            )?;
                            builder.ins().iadd(lhs, rhs)
                        }
                        BinaryOperator::Subtract => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "subtract lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "subtract rhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "subtract result",
                            )?;
                            builder.ins().isub(lhs, rhs)
                        }
                        BinaryOperator::Multiply => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "multiply lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "multiply rhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "multiply result",
                            )?;
                            builder.ins().imul(lhs, rhs)
                        }
                        BinaryOperator::Divide => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "divide lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "divide rhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "divide result",
                            )?;
                            builder.ins().sdiv(lhs, rhs)
                        }
                        BinaryOperator::Modulo => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "modulo lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "modulo rhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "modulo result",
                            )?;
                            builder.ins().srem(lhs, rhs)
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
                            }
                        }
                        BinaryOperator::GreaterThanOrEqual => {
                            match self.require_ordered_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )? {
                                NativeCompareKind::Integer => builder
                                    .ins()
                                    .icmp(IntCC::SignedGreaterThanOrEqual, lhs, rhs),
                                NativeCompareKind::Float => {
                                    builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs)
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
                            }
                        }
                        BinaryOperator::Equals => {
                            let shape = self.require_equatable_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )?;
                            self.lower_native_equality_shape(
                                kernel_id, expr_id, &shape, lhs, rhs, builder,
                            )?
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
                                NativeEqualityShape::Text
                                | NativeEqualityShape::Bytes
                                | NativeEqualityShape::Aggregate(_)
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
                        crate::InlinePipeStageKind::Debug { .. } => {
                            return Err(self.unsupported_inline_pipe_stage(
                                kernel_id,
                                pipe_expr,
                                stage_index,
                                "still requires runtime-side debug effects",
                            ));
                        }
                        crate::InlinePipeStageKind::Gate { .. } => {
                            return Err(self.unsupported_inline_pipe_stage(
                                kernel_id,
                                pipe_expr,
                                stage_index,
                                "still requires control-flow/Option branching codegen",
                            ));
                        }
                        crate::InlinePipeStageKind::Case { .. } => {
                            return Err(self.unsupported_inline_pipe_stage(
                                kernel_id,
                                pipe_expr,
                                stage_index,
                                "still requires pattern-matching codegen",
                            ));
                        }
                        crate::InlinePipeStageKind::TruthyFalsy { .. } => {
                            return Err(self.unsupported_inline_pipe_stage(
                                kernel_id,
                                pipe_expr,
                                stage_index,
                                "still requires branch selection codegen",
                            ));
                        }
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

    fn require_compilable_item_call(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        item: ItemId,
        arguments: &[KernelExprId],
    ) -> Result<KernelId, CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        let item_decl = self
            .program
            .items()
            .get(item)
            .expect("validated backend kernels keep item references aligned with codegen");
        if matches!(item_decl.kind, crate::ItemKind::Signal(_)) {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "signal item `{}` still requires signal-aware item codegen",
                    item_decl.name
                ),
            ));
        }
        let Some(body) = item_decl.body else {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!("item `{}` has no body kernel to compile", item_decl.name),
            ));
        };
        if arguments.is_empty() {
            if !item_decl.parameters.is_empty() {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "item `{}` expects {} argument(s) and still requires callable codegen when referenced without saturation",
                        item_decl.name,
                        item_decl.parameters.len()
                    ),
                ));
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
        Ok(body)
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
                let body = self.require_compilable_item_call(kernel_id, expr_id, *item, arguments)?;
                Ok(DirectApplyPlan::Item { body })
            }
            KernelExprKind::DomainMember(handle) => self
                .require_compilable_domain_member_call(kernel_id, expr_id, callee, handle, arguments)
                .map(DirectApplyPlan::DomainMember),
            KernelExprKind::Builtin(term) => self
                .require_compilable_builtin_call(kernel_id, expr_id, callee, *term, arguments)
                .map(DirectApplyPlan::Builtin),
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
                "the current Cranelift slice only lowers direct saturated item calls, selected direct bytes intrinsics, representational by-reference domain-member calls, and niche Option constructors",
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
                self.require_niche_option_expression(
                    kernel_id,
                    kernel,
                    expr_id,
                    Some(*payload),
                    result_layout,
                    &detail,
                )?;
                Ok(BuiltinCallPlan::NicheOptionSome)
            }
            BuiltinTerm::None
            | BuiltinTerm::Ok
            | BuiltinTerm::Err
            | BuiltinTerm::Valid
            | BuiltinTerm::Invalid => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "builtin constructor `{term}` still requires backend-owned aggregate constructor lowering; the current Cranelift slice only lowers Bool literals plus niche Option None/Some forms"
                ),
            )),
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
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} still requires backend-owned bytes/runtime lowering beyond the current empty/length/fromText Cranelift subset"
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
            DirectApplyPlan::DomainMember(DomainMemberCallPlan::RepresentationalIdentityUnary)
            | DirectApplyPlan::Builtin(BuiltinCallPlan::NicheOptionSome)
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
            DirectApplyPlan::Intrinsic(IntrinsicCallPlan::BytesLength) => {
                let [argument] = arguments else {
                    return Err(self.unsupported_expression(
                        kernel_id,
                        expr_id,
                        "direct bytes.length lowering expected exactly one materialized argument",
                    ));
                };
                Ok(builder.ins().load(types::I64, MemFlags::new(), *argument, 0))
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

    fn require_niche_option_expression(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        payload: Option<KernelExprId>,
        layout: LayoutId,
        detail: &str,
    ) -> Result<(), CodegenError> {
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
        if self.program.layouts()[*element].abi != AbiPassMode::ByReference {
            return Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "{detail} currently requires an Option over a by-reference payload so codegen can use a null-pointer niche, found payload layout{element}=`{}`",
                    self.program.layouts()[*element]
                ),
            ));
        }
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
        Ok(())
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
            (LayoutKind::Primitive(PrimitiveType::Int), LayoutKind::Primitive(PrimitiveType::Int)) => {
                NativeCompareKind::Integer
            }
            (
                LayoutKind::Primitive(PrimitiveType::Float),
                LayoutKind::Primitive(PrimitiveType::Float),
            ) => NativeCompareKind::Float,
            _ => {
                return Err(self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    &format!(
                        "comparison expects matching Int/Float operands, found layout{left_layout_id}=`{left_layout}` and layout{right_layout_id}=`{right_layout}`"
                    ),
                ));
            }
        };
        self.require_bool_expression(kernel_id, expr_id, kernel.exprs()[expr_id].layout, "comparison result")?;
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
        let kind = self.resolve_native_equality_shape(
            kernel_id,
            expr_id,
            left_layout_id,
            &mut visited,
        )?;
        self.require_bool_expression(kernel_id, expr_id, kernel.exprs()[expr_id].layout, "equality result")?;
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
                    "equality for layout{layout}=`{}` still requires a compiled representation bridge beyond native scalar/Text/Bytes/record/tuple/niche-Option shapes",
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
            NativeEqualityShape::Text | NativeEqualityShape::Bytes => {
                Ok(self.lower_native_byte_sequence_equality(lhs, rhs, builder))
            }
            NativeEqualityShape::Aggregate(fields) => {
                let mut equal = builder.ins().iconst(types::I8, 1);
                for field in fields {
                    let abi = self.field_abi_shape(kernel_id, field.layout, "native equality field")?;
                    let left_field = builder
                        .ins()
                        .load(abi.ty, MemFlags::new(), lhs, field.offset);
                    let right_field = builder
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
                builder
                    .ins()
                    .brif(
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
                builder.ins().jump(merge_block, &[BlockArg::Value(some_equal)]);

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
        let right_byte = builder.ins().load(types::I8, MemFlags::new(), right_addr, 0);
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

    fn materialize_text_literal(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        text: &crate::TextLiteral,
        builder: &mut FunctionBuilder<'_>,
    ) -> Result<Value, CodegenError> {
        let rendered = self
            .render_static_text_literal(kernel_id, kernel, expr_id, text)?
            .ok_or_else(|| {
                self.unsupported_expression(
                    kernel_id,
                    expr_id,
                    "text interpolation still requires a native text formatting contract beyond static literal folding",
                )
            })?;
        self.materialize_text_constant(kernel_id, rendered.as_ref(), builder)
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
        self.materialize_byte_sequence_constant(kernel_id, "text_literal", rendered.as_bytes(), builder)
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
        self.materialize_literal_pointer(
            kernel_id,
            family,
            encoded.into_boxed_slice(),
            8,
            builder,
        )
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
        Ok(Some(self.materialize_static_plan(kernel_id, plan, builder)?))
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
        }
    }

    fn require_static_scalar_aggregate_expression(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
    ) -> Result<(), CodegenError> {
        let _ = self.encode_static_scalar_aggregate_constant(kernel_id, kernel, expr_id)?;
        Ok(())
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
            let abi = self.field_abi_shape(
                kernel_id,
                layout,
                &format!("{detail} field `{label}`"),
            )?;
            max_align = max_align.max(abi.align);
            offset = align_to(offset, abi.align);
            encoded.resize(offset as usize, 0);
            encoded.extend_from_slice(
                &self.encode_static_scalar_field(kernel_id, expr_id, layout, value, detail, &label)?,
            );
            offset = offset.checked_add(abi.size).ok_or_else(|| CodegenError::UnsupportedLayout {
                kernel: kernel_id,
                layout,
                detail: format!("{detail} field `{label}` overflows backend constant packing")
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
                Task::Visit(expr_id) => match &kernel.exprs()[expr_id].kind {
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
                        let value = integer.raw.parse::<i64>().map(RuntimeValue::Int).map_err(|_| {
                            CodegenError::InvalidIntegerLiteral {
                                kernel: kernel_id,
                                expr: expr_id,
                                raw: integer.raw.clone(),
                            }
                        })?;
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
                    KernelExprKind::Apply { callee, arguments } => match &kernel.exprs()[*callee].kind {
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
                                KernelExprKind::Builtin(BuiltinTerm::Ok) => crate::RuntimeConstructor::Ok,
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
                            let Some(expected_arity) = static_intrinsic_arity(*intrinsic) else {
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
                    },
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
                },
                Task::BuildOptionSome => {
                    let payload = values.pop().expect("static option payload should exist");
                    values.push(RuntimeValue::OptionSome(Box::new(payload)));
                }
                Task::BuildBuiltinConstructor { constructor } => {
                    let payload = values.pop().expect("static constructor payload should exist");
                    let value = match constructor {
                        crate::RuntimeConstructor::Some => RuntimeValue::OptionSome(Box::new(payload)),
                        crate::RuntimeConstructor::Ok => RuntimeValue::ResultOk(Box::new(payload)),
                        crate::RuntimeConstructor::Err => RuntimeValue::ResultErr(Box::new(payload)),
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
                            static_intrinsic_arity(intrinsic)
                                .expect("static intrinsic builder should only use supported arities"),
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
                    let right = static_strip_signal(
                        values.pop().expect("static binary rhs should exist"),
                    );
                    let left = static_strip_signal(
                        values.pop().expect("static binary lhs should exist"),
                    );
                    let Some(value) = (match (operator, &left, &right) {
                        (BinaryOperator::GreaterThan, RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                            Some(RuntimeValue::Bool(left > right))
                        }
                        (BinaryOperator::GreaterThan, RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                            Some(RuntimeValue::Bool(left.to_f64() > right.to_f64()))
                        }
                        (BinaryOperator::LessThan, RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                            Some(RuntimeValue::Bool(left < right))
                        }
                        (BinaryOperator::LessThan, RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                            Some(RuntimeValue::Bool(left.to_f64() < right.to_f64()))
                        }
                        (BinaryOperator::GreaterThanOrEqual, RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                            Some(RuntimeValue::Bool(left >= right))
                        }
                        (BinaryOperator::GreaterThanOrEqual, RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                            Some(RuntimeValue::Bool(left.to_f64() >= right.to_f64()))
                        }
                        (BinaryOperator::LessThanOrEqual, RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                            Some(RuntimeValue::Bool(left <= right))
                        }
                        (BinaryOperator::LessThanOrEqual, RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                            Some(RuntimeValue::Bool(left.to_f64() <= right.to_f64()))
                        }
                        (BinaryOperator::And, RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => {
                            Some(RuntimeValue::Bool(*left && *right))
                        }
                        (BinaryOperator::Or, RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => {
                            Some(RuntimeValue::Bool(*left || *right))
                        }
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
                        .filter(|segment| matches!(segment, crate::TextSegment::Interpolation { .. }))
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
        IntrinsicValue::BytesLength | IntrinsicValue::BytesFromText | IntrinsicValue::BytesToText => {
            Some(1)
        }
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
        (IntrinsicValue::BytesGet, [RuntimeValue::Int(index), RuntimeValue::Bytes(bytes)]) => {
            Some(
                usize::try_from(*index)
                    .ok()
                    .and_then(|index| bytes.get(index))
                    .map(|&byte| RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(byte as i64))))
                    .unwrap_or(RuntimeValue::OptionNone),
            )
        }
        (
            IntrinsicValue::BytesSlice,
            [RuntimeValue::Int(from), RuntimeValue::Int(to), RuntimeValue::Bytes(bytes)],
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
        (RuntimeValue::Set(left), RuntimeValue::Set(right)) => static_unordered_values_eq(left, right),
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
        | (RuntimeValue::Signal(left), RuntimeValue::Signal(right)) => static_structural_eq(left, right),
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
