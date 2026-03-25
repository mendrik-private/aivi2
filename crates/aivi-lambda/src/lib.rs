#![forbid(unsafe_code)]

//! Closed typed-lambda foundations for the AIVI compiler.
//!
//! This crate sits between `aivi-core` and `aivi-backend` and keeps the lambda/backend boundary
//! honest for the currently proven compiler slice:
//! - validated typed-core expressions remain the body language for now,
//! - closure boundaries become explicit module-owned nodes,
//! - captured environments are analyzed once and carried as stable lambda metadata,
//! - ordinary item bodies and backend-consumable runtime bodies use one closure abstraction,
//! - and structural validation plus a stable debug surface keep the layer real.
//!
//! The current slice is intentionally narrow. It makes closure/environment structure explicit
//! without pretending to have already chosen backend ABI, layout, or codegen details.

mod analysis;
mod ids;
mod lower;
mod module;
mod validate;

pub use aivi_core::{Arena, ArenaId, ArenaOverflow};
pub use aivi_core::{
    DecodeProgram, DecodeProgramId, DecodeStep, DecodeStepId, DomainDecodeSurface,
    DomainDecodeSurfaceKind, Expr, ExprId, FanoutStage, ItemId, ItemKind, ItemParameter, PipeId,
    PipeOrigin, SignalInfo, SourceId, SourceNode, StageId, TruthyFalsyStage, Type,
};
pub use aivi_typing::{NonSourceWakeupCause, RecurrencePlan, RecurrenceWakeupPlan};
pub use ids::{CaptureId, ClosureId};
pub use lower::{LoweringError, LoweringErrors, lower_module};
pub use module::{
    Capture, Closure, ClosureKind, GateStage, Item, Module, NonSourceWakeup, Pipe, PipeRecurrence,
    RecurrenceStage, Stage, StageKind,
};
pub use validate::{ClosureMetadataField, ValidationError, ValidationErrors, validate_module};
