use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

use cranelift_codegen::{
    ir::{AbiParam, InstBuilder, MemFlags, Type, UserFuncName, Value, condcodes::IntCC, types},
    print_errors::pretty_verifier_error,
    settings, verify_function,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::{
    AbiPassMode, BinaryOperator, BuiltinTerm, CallingConventionKind, EnvSlotId, Kernel,
    KernelExprId, KernelExprKind, KernelId, KernelOriginKind, LayoutId, LayoutKind, ParameterRole,
    PrimitiveType, Program, SubjectRef, UnaryOperator, ValidationError, describe_expr_kind,
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
/// - it materializes `Int` as `i64`, `Bool` as `i8`, and backend by-reference values as host
///   pointers,
/// - it uses a backend-local pointer niche for `Option` over by-reference payloads,
/// - it resolves record projection offsets inside backend/codegen,
/// - and it explicitly rejects general apply/domain/collection/text/inline-pipe lowering until
///   those contracts are owned in this layer.
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
    function_builder_ctx: FunctionBuilderContext,
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
            function_builder_ctx: FunctionBuilderContext::new(),
        })
    }

    fn prevalidate(&self) -> Result<(), CodegenErrors> {
        let mut errors = Vec::new();

        for (kernel_id, kernel) in self.program.kernels().iter() {
            if matches!(kernel.origin.kind, KernelOriginKind::ItemBody) {
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
                match &expr.kind {
                    KernelExprKind::Subject(SubjectRef::Input) | KernelExprKind::Environment(_) => {
                    }
                    KernelExprKind::Subject(SubjectRef::Inline(_)) => {
                        errors.push(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            "inline subjects require inline-pipe codegen, which stays out of this backend slice",
                        ));
                    }
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
                    KernelExprKind::Projection { base, .. } => {
                        let Some(base_layout) = (match base {
                            crate::ProjectionBase::Subject(SubjectRef::Input) => {
                                kernel.input_subject
                            }
                            crate::ProjectionBase::Subject(SubjectRef::Inline(_)) => {
                                errors.push(self.unsupported_expression(
                                    kernel_id,
                                    expr_id,
                                    "projection from inline subjects still requires inline-pipe codegen",
                                ));
                                None
                            }
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
                            BinaryOperator::Add | BinaryOperator::Subtract => {
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
                            BinaryOperator::GreaterThan | BinaryOperator::LessThan => {
                                if let Err(error) = self.require_int_expression(
                                    kernel_id,
                                    *left,
                                    kernel.exprs()[*left].layout,
                                    "comparison lhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_int_expression(
                                    kernel_id,
                                    *right,
                                    kernel.exprs()[*right].layout,
                                    "comparison rhs",
                                ) {
                                    errors.push(error);
                                }
                                if let Err(error) = self.require_bool_expression(
                                    kernel_id,
                                    expr_id,
                                    expr.layout,
                                    "comparison result",
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
                    KernelExprKind::Item(_)
                    | KernelExprKind::DomainMember(_)
                    | KernelExprKind::Builtin(_)
                    | KernelExprKind::SuffixedInteger(_)
                    | KernelExprKind::Text(_)
                    | KernelExprKind::Tuple(_)
                    | KernelExprKind::List(_)
                    | KernelExprKind::Map(_)
                    | KernelExprKind::Set(_)
                    | KernelExprKind::Record(_)
                    | KernelExprKind::Apply { .. }
                    | KernelExprKind::Pipe(_) => {
                        errors.push(self.unsupported_expression(
                            kernel_id,
                            expr_id,
                            "the current Cranelift slice lowers record projection, pointer-niche Option carriers, scalar literals, and unary/binary Int/Bool operators only",
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
        let mut compiled_kernels = Vec::with_capacity(self.program.kernels().len());
        let mut errors = Vec::new();

        for (kernel_id, kernel) in self.program.kernels().iter() {
            if matches!(kernel.origin.kind, KernelOriginKind::ItemBody) {
                continue;
            }
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

    fn compile_kernel(
        &mut self,
        kernel_id: KernelId,
        kernel: &Kernel,
    ) -> Result<CompiledKernel, CodegenError> {
        match kernel.convention.kind {
            CallingConventionKind::RuntimeKernelV1 => {}
        }

        let signature = self.build_signature(kernel_id, kernel)?;
        let symbol = kernel_symbol(self.program, kernel_id, kernel);
        let func_id = self
            .module
            .declare_function(&symbol, Linkage::Local, &signature)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: error.to_string().into_boxed_str(),
            })?;

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
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        builder: &mut FunctionBuilder<'_>,
        entry: cranelift_codegen::ir::Block,
    ) -> Result<Value, CodegenError> {
        enum Task {
            Visit(KernelExprId),
            BuildOptionSome(KernelExprId),
            BuildProjection(KernelExprId),
            BuildUnary(KernelExprId),
            BuildBinary(KernelExprId),
        }

        let mut input = None;
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
                    match &expr.kind {
                        KernelExprKind::Subject(SubjectRef::Input) => {
                            let Some(value) = input else {
                                return Err(CodegenError::MissingInputParameter {
                                    kernel: kernel_id,
                                });
                            };
                            values.push(value);
                        }
                        KernelExprKind::Subject(SubjectRef::Inline(_)) => {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "inline subjects require inline-pipe codegen, which stays out of this first scalar slice",
                            ));
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
                        KernelExprKind::Builtin(BuiltinTerm::True) => {
                            self.require_bool_expression(kernel_id, expr_id, expr.layout, "True")?;
                            values.push(builder.ins().iconst(types::I8, 1));
                        }
                        KernelExprKind::Builtin(BuiltinTerm::False) => {
                            self.require_bool_expression(kernel_id, expr_id, expr.layout, "False")?;
                            values.push(builder.ins().iconst(types::I8, 0));
                        }
                        KernelExprKind::Projection { base, .. } => match base {
                            crate::ProjectionBase::Subject(SubjectRef::Input) => {
                                let Some(value) = input else {
                                    return Err(CodegenError::MissingInputParameter {
                                        kernel: kernel_id,
                                    });
                                };
                                let base_layout = kernel.input_subject.expect(
                                    "validated backend kernels keep input subjects aligned with codegen",
                                );
                                values.push(self.lower_projection(
                                    kernel_id,
                                    kernel,
                                    expr_id,
                                    value,
                                    base_layout,
                                    builder,
                                )?);
                            }
                            crate::ProjectionBase::Subject(SubjectRef::Inline(_)) => {
                                return Err(self.unsupported_expression(
                                    kernel_id,
                                    expr_id,
                                    "projection from inline subjects still requires inline-pipe codegen",
                                ));
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
                        _ => {
                            return Err(self.unsupported_expression(
                                kernel_id,
                                expr_id,
                                "the current Cranelift slice only lowers record projection, pointer-niche Option carriers, scalar subjects/environment slots, integers, booleans, and unary/binary operators",
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
                        BinaryOperator::GreaterThan => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "comparison lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "comparison rhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "comparison result",
                            )?;
                            builder.ins().icmp(IntCC::SignedGreaterThan, lhs, rhs)
                        }
                        BinaryOperator::LessThan => {
                            self.require_int_expression(
                                kernel_id,
                                *left,
                                kernel.exprs()[*left].layout,
                                "comparison lhs",
                            )?;
                            self.require_int_expression(
                                kernel_id,
                                *right,
                                kernel.exprs()[*right].layout,
                                "comparison rhs",
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "comparison result",
                            )?;
                            builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs)
                        }
                        BinaryOperator::Equals => {
                            self.require_equatable_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "equality result",
                            )?;
                            builder.ins().icmp(IntCC::Equal, lhs, rhs)
                        }
                        BinaryOperator::NotEquals => {
                            self.require_equatable_expression_pair(
                                kernel_id, kernel, expr_id, *left, *right,
                            )?;
                            self.require_bool_expression(
                                kernel_id,
                                expr_id,
                                expr.layout,
                                "inequality result",
                            )?;
                            builder.ins().icmp(IntCC::NotEqual, lhs, rhs)
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
            }
        }

        Ok(values
            .pop()
            .expect("kernel expression lowering should leave one root value"))
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

    fn require_equatable_expression_pair(
        &self,
        kernel_id: KernelId,
        kernel: &Kernel,
        expr_id: KernelExprId,
        left: KernelExprId,
        right: KernelExprId,
    ) -> Result<(), CodegenError> {
        let left_layout = self.program.layouts()[kernel.exprs()[left].layout].clone();
        let right_layout = self.program.layouts()[kernel.exprs()[right].layout].clone();
        let left_layout_id = kernel.exprs()[left].layout;
        let right_layout_id = kernel.exprs()[right].layout;
        match (&left_layout.kind, &right_layout.kind) {
            (LayoutKind::Primitive(PrimitiveType::Int), LayoutKind::Primitive(PrimitiveType::Int))
            | (
                LayoutKind::Primitive(PrimitiveType::Bool),
                LayoutKind::Primitive(PrimitiveType::Bool),
            ) => Ok(()),
            _ => Err(self.unsupported_expression(
                kernel_id,
                expr_id,
                &format!(
                    "equality expects matching Int/Bool operands, found layout{left_layout_id}=`{left_layout}` and layout{right_layout_id}=`{right_layout}`"
                ),
            )),
        }
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
                LayoutKind::Primitive(PrimitiveType::Bool) => Ok(AbiShape {
                    ty: types::I8,
                    size: 1,
                    align: 1,
                }),
                LayoutKind::Primitive(other) => Err(CodegenError::UnsupportedLayout {
                    kernel: kernel_id,
                    layout,
                    detail: format!(
                        "{detail} uses primitive `{other}`, but the current Cranelift slice only materializes Int and Bool by value"
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
            KernelOriginKind::ItemBody => "item_body".to_owned(),
            KernelOriginKind::GateTrue { stage_index, .. } => format!("gate_true_s{stage_index}"),
            KernelOriginKind::GateFalse { stage_index, .. } => format!("gate_false_s{stage_index}"),
            KernelOriginKind::SignalFilterPredicate { stage_index, .. } => {
                format!("signal_filter_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStart { stage_index, .. } => {
                format!("recurrence_start_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStep { stage_index, .. } => {
                format!("recurrence_step_s{stage_index}")
            }
            KernelOriginKind::RecurrenceWakeupWitness { .. } => "recurrence_witness".to_owned(),
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
