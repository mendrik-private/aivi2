use std::{collections::BTreeMap, fs, path::PathBuf};

use aivi_backend::{
    AbiPassMode, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject,
    BuiltinTerm, BuiltinTraversableCarrier, CodegenError, DecodeStepKind, DomainDecodeSurfaceKind,
    EvaluationError, ExecutableEvidence, GateStage as BackendGateStage, InlinePipeConstructor,
    InlinePipePatternKind, InlinePipeStageKind, ItemKind as BackendItemKind, KernelEvaluator,
    KernelExprKind, KernelOriginKind, LayoutKind, LoweringError, NonSourceWakeupCause,
    ProjectionBase, RecurrenceTarget, RuntimeBigInt, RuntimeDbCommitPlan, RuntimeDbConnection,
    RuntimeDbQueryPlan, RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeDecimal, RuntimeFloat,
    RuntimeRecordField, RuntimeSumValue, RuntimeTaskPlan, RuntimeValue, SourceProvider,
    StageKind as BackendStageKind, SubjectRef, ValidationError, compile_program,
    lower_module as lower_backend_module, validate_program,
};
use aivi_base::{SourceDatabase, SourceSpan};
use aivi_core::{
    Expr as CoreExpr, ExprKind as CoreExprKind, GateStage as CoreGateStage, Item as CoreItem,
    ItemKind as CoreItemKind, ItemParameter as CoreItemParameter, Module as CoreModule,
    Pipe as CorePipe, PipeExpr as CoreInlinePipeExpr, PipeOrigin as CorePipeOrigin,
    PipeStage as CoreInlinePipeStage, PipeStageKind as CoreInlinePipeStageKind,
    ProjectionBase as CoreProjectionBase, RecordField as CoreRecordField,
    Reference as CoreReference, Stage as CoreStage, StageKind as CoreStageKind, Type as CoreType,
    lower_module as lower_core_module, validate_module as validate_core_module,
};
use aivi_hir::{
    BigIntLiteral, BinaryOperator as HirBinaryOperator, BindingId as HirBindingId,
    BuiltinTerm as HirBuiltinTerm, BuiltinType, DecimalLiteral, ExprId as HirExprId, FloatLiteral,
    IntegerLiteral, ItemId as HirItemId, PipeTransformMode, SourceProviderRef,
    SourceReplacementPolicy as HirSourceReplacementPolicy,
    SourceStaleWorkPolicy as HirSourceStaleWorkPolicy,
    SourceTeardownPolicy as HirSourceTeardownPolicy, TypeParameterId as HirTypeParameterId,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::RootDatabase;
use aivi_syntax::parse_module;
use aivi_typing::SourceCancellationPolicy as TypingSourceCancellationPolicy;

include!("foundations_parts/support.rs");
include!("foundations_parts/lowering.rs");
include!("foundations_parts/runtime_eval.rs");
include!("foundations_parts/inline_pipes.rs");
include!("foundations_parts/domains.rs");
include!("foundations_parts/validation.rs");
include!("foundations_parts/codegen_core.rs");
include!("foundations_parts/codegen_equality.rs");
include!("foundations_parts/db_numeric_misc.rs");
include!("foundations_parts/patterns_recurrence.rs");
