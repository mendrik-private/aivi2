use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::Hash,
    time::{Duration, Instant},
};

use indexmap::IndexMap;

use aivi_hir::{DomainMemberHandle, IntrinsicValue, ItemId as HirItemId, SumConstructorHandle};

use crate::{
    BinaryOperator, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject,
    BuiltinTerm, BuiltinTraversableCarrier, EnvSlotId, InlinePipeConstructor, InlinePipePattern,
    InlinePipePatternKind, InlinePipeStageKind, InlineSubjectId, ItemId, KernelExprId,
    KernelExprKind, KernelId, LayoutId, LayoutKind, PrimitiveType, Program, ProjectionBase,
    SubjectRef, UnaryOperator,
    numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat},
};

include!("values.rs");
include!("errors_profiles.rs");
include!("evaluator.rs");
include!("intrinsics.rs");
include!("equality.rs");

#[cfg(test)]
mod tests;
