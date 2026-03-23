#![forbid(unsafe_code)]

//! First typed-core foundations for the AIVI compiler.
//!
//! This crate owns a genuine post-HIR intermediate representation for the already-proven reactive
//! and source-facing frontend slice:
//! - core-owned type and expression nodes,
//! - typed `val` / `fun` bodies with explicit local parameters,
//! - deterministic typed arenas and ids,
//! - normalized pipe-stage plans for gates, truthy/falsy pairs, fanout, and recurrence,
//! - source lifecycle and decode-program nodes,
//! - and structural validation plus a stable pretty/debug surface.
//!
//! The current slice is intentionally narrow. It consumes only HIR elaboration reports the frontend
//! can already justify today and rejects blocked handoffs explicitly instead of guessing missing
//! core semantics.

pub mod arena;
pub mod expr;
pub mod ids;
mod lower;
mod module;
pub mod ty;
mod validate;

pub use arena::{Arena, ArenaId, ArenaOverflow};
pub use expr::{
    Expr, ExprKind, MapEntry, Pattern, PatternBinding, PatternConstructor, PatternKind,
    PipeCaseArm, PipeExpr, PipeStage, PipeStageKind, PipeTruthyFalsyBranch, PipeTruthyFalsyStage,
    ProjectionBase, RecordExprField, RecordPatternField, Reference, TextLiteral, TextSegment,
};
pub use ids::{DecodeProgramId, DecodeStepId, ExprId, ItemId, PipeId, SourceId, StageId};
pub use lower::{LoweringError, LoweringErrors, lower_module};
pub use module::{
    DecodeField, DecodeProgram, DecodeStep, DecodeVariant, DomainDecodeSurface,
    DomainDecodeSurfaceKind, FanoutFilter, FanoutJoin, FanoutStage, GateStage, Item, ItemKind,
    ItemParameter, Module, NonSourceWakeup, Pipe, PipeOrigin, PipeRecurrence, RecurrenceGuard,
    RecurrenceStage, SignalInfo, SourceArgumentValue, SourceInstanceId, SourceNode,
    SourceOptionBinding, SourceOptionValue, Stage, StageKind, TruthyFalsyBranch, TruthyFalsyStage,
};
pub use ty::{RecordField, Type};
pub use validate::{ValidationError, ValidationErrors, validate_module};
