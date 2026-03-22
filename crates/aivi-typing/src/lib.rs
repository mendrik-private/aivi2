#![forbid(unsafe_code)]

//! Milestone 3 type-side semantics, starting with explicit `Eq` derivation plans.
//!
//! The crate intentionally models only the structural fragment needed for compiler-derived
//! equality plus named leaf references supplied by the caller. It does not pretend full
//! instance resolution or runtime evaluation already exists.

pub mod eq;

pub use eq::{
    Class, Closedness, EqContext, EqDerivation, EqDerivationError, EqDerivationErrorKind,
    EqDeriver, EqFieldPlan, EqPathSegment, EqPlanId, EqStep, EqVariantPlan, ExternalTypeId,
    FieldName, InstanceHead, PrimitiveType, RecordField, RecordShape, ShapeError, ShapeErrorKind,
    SumShape, SumVariant, TypeId, TypeNode, TypeParameterId, TypeReference, TypeStore, VariantName,
};
