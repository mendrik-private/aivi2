#![forbid(unsafe_code)]

//! First typed-core foundations for the AIVI compiler.
//!
//! This crate owns a genuine post-HIR intermediate representation for the already-proven reactive
//! and source-facing frontend slice:
//! - core-owned type and expression nodes,
//! - typed `value` / `func` bodies with explicit local parameters,
//! - deterministic typed arenas and ids,
//! - normalized pipe-stage plans for gates, truthy/falsy pairs, fanout, and recurrence,
//! - source lifecycle and decode-program nodes,
//! - and structural validation plus a stable pretty/debug surface.
//!
//! The current slice is intentionally narrow. It consumes only HIR elaboration reports the frontend
//! can already justify today and rejects blocked handoffs explicitly instead of guessing missing
//! core semantics.

use std::collections::HashSet;

pub mod arena;
mod class_support;
pub mod expr;
pub mod ids;
mod lower;
mod module;
pub mod ty;
mod validate;

pub use arena::{Arena, ArenaId, ArenaOverflow, alloc_or_diag};
pub use class_support::{
    BuiltinExecutableCarrier, BuiltinExecutableClass, BuiltinExecutableClassSupport,
    HIGHER_KINDED_DOC_CARRIERS, HIGHER_KINDED_DOC_CLASSES, TRAVERSE_RESULT_APPLICATIVE_CARRIERS,
    builtin_append_intrinsic, builtin_apply_intrinsic, builtin_bimap_intrinsic,
    builtin_chain_intrinsic, builtin_compare_intrinsic, builtin_empty_intrinsic,
    builtin_executable_class_support, builtin_filter_map_intrinsic, builtin_join_intrinsic,
    builtin_map_intrinsic, builtin_pure_intrinsic, builtin_reduce_intrinsic,
    builtin_traverse_intrinsic, builtin_traverse_result_applicative_support,
    render_higher_kinded_builtin_support_markdown,
};
pub use expr::{
    BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier, BuiltinBifunctorCarrier,
    BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier, BuiltinFoldableCarrier,
    BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject, BuiltinTraversableCarrier,
    ExecutableClassMember, ExecutableEvidence, Expr, ExprKind, MapEntry, Pattern, PatternBinding,
    PatternConstructor, PatternKind, PipeCaseArm, PipeExpr, PipeStage, PipeStageKind,
    PipeTruthyFalsyBranch, PipeTruthyFalsyStage, ProjectionBase, RecordExprField,
    RecordPatternField, Reference, TextLiteral, TextSegment,
};
pub use ids::{DecodeProgramId, DecodeStepId, ExprId, ItemId, PipeId, SourceId, StageId};
pub use lower::{
    LoweredRuntimeFragment, LoweringError, LoweringErrors, RuntimeFragmentSpec, lower_module,
    lower_module_with_items, lower_runtime_fragment, lower_runtime_module,
    lower_runtime_module_with_items, lower_runtime_module_with_workspace,
    runtime_fragment_included_items,
};
pub use module::{
    DecodeField, DecodeProgram, DecodeStep, DecodeVariant, DomainDecodeSurface,
    DomainDecodeSurfaceKind, FanoutFilter, FanoutJoin, FanoutStage, GateStage, Item, ItemKind,
    ItemParameter, Module, NonSourceWakeup, Pipe, PipeOrigin, PipeRecurrence, RecurrenceGuard,
    RecurrenceStage, SignalInfo, SourceArgumentValue, SourceInstanceId, SourceNode,
    SourceOptionBinding, SourceOptionValue, Stage, StageKind, TemporalStage, TruthyFalsyBranch,
    TruthyFalsyStage,
};
pub use ty::{RecordField, Type};
pub use validate::{ValidationError, ValidationErrors, validate_module};

pub type IncludedItems = HashSet<aivi_hir::ItemId>;
