#![forbid(unsafe_code)]

//! GTK bridge foundations for the AIVI widget runtime.
//!
//! This crate lowers HIR markup into a stable widget/control graph with explicit property setters,
//! event hookups, child operations, and control-node branches, adapts that plan into a typed
//! runtime-facing owner/input assembly, lowers that assembly into a GTK-oriented bridge graph, then
//! executes that graph through an explicit GTK-host boundary without inventing VDOM semantics.

pub mod bridge;
pub mod executor;
pub mod lower;
pub mod plan;
pub mod runtime_adapter;

pub use bridge::*;
pub use executor::*;
pub use lower::{
    LoweringError, LoweringOptions, lower_markup_expr, lower_markup_expr_with_options,
    lower_markup_root, lower_markup_root_with_options,
};
pub use plan::*;
pub use runtime_adapter::{
    RuntimeCaseBranch, RuntimeCaseNode, RuntimeChildOp, RuntimeEachNode, RuntimeEmptyNode,
    RuntimeEventBinding, RuntimeExprInput, RuntimeFragmentNode, RuntimeMatchNode, RuntimeNodeRef,
    RuntimePlanNode, RuntimePlanNodeKind, RuntimePropertyBinding, RuntimeSetterBinding,
    RuntimeShowMountPolicy, RuntimeShowNode, RuntimeWidgetNode, RuntimeWithNode,
    WidgetRuntimeAdapterError, WidgetRuntimeAdapterErrors, WidgetRuntimeAssembly,
    WidgetRuntimeAssemblyBuilder, assemble_widget_runtime,
};
