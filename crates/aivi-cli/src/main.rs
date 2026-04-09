#![forbid(unsafe_code)]

mod manual_snippets;
mod mcp;
mod run_session;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
    sync::{Arc, mpsc as sync_mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use aivi_backend::{
    BackendExecutableProgram, BackendExecutionEngineHandle, DetachedRuntimeValue,
    ItemId as BackendItemId, KernelEvaluationProfile, Program as BackendProgram, RuntimeFloat,
    RuntimeRecordField, RuntimeValue, cache::compute_program_fingerprint, compile_program_cached,
    lower_module as lower_backend_module, validate_program,
};
use aivi_base::{Diagnostic, FileId, Severity, SourceDatabase, SourceSpan};
use aivi_core::{
    IncludedItems, RuntimeFragmentSpec, lower_runtime_fragment, lower_runtime_module_with_items,
    lower_runtime_module_with_workspace, runtime_fragment_included_items,
    validate_module as validate_core_module,
};
use aivi_gtk::{
    GtkBridgeGraph, GtkBridgeNodeKind, GtkBridgeNodeRef, GtkChildGroup, GtkCollectionKey,
    GtkConcreteEventPayload, GtkConcreteHost, GtkExecutionPath, GtkHostValue, GtkNodeInstance,
    GtkRuntimeExecutor, RepeatedChildPolicy, RuntimePropertyBinding, RuntimeShowMountPolicy,
    SetterSource, lookup_widget_event, lookup_widget_schema, lower_markup_expr,
    lower_widget_bridge,
};
use aivi_hir::{
    BuiltinTerm, BuiltinType, DecoratorPayload, ExprId as HirExprId, ExprKind, GateRecordField,
    GateType, GeneralExprOutcome, GeneralExprParameter, ImportBindingMetadata, ImportId,
    ImportValueType, Item, ItemId as HirItemId, MarkupRuntimeExprSites, Module as HirModule,
    PatternId as HirPatternId, PatternKind, TermResolution, ValidationMode, ValueItem,
    collect_markup_runtime_expr_sites, elaborate_runtime_expr_with_env, signal_payload_type,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::{
    HirModuleResult, QueryCacheStats, RootDatabase, SourceFile as QuerySourceFile,
    discover_workspace_root_from_directory, hir_module as query_hir_module, parse_manifest,
    parsed_file as query_parsed_file, resolve_v1_entrypoint, runtime_fragment_backend_unit,
    whole_program_backend_unit_with_items,
};
use aivi_runtime::{
    BackendLinkedRuntime, GlibLinkedRuntimeDriver, GlibLinkedRuntimeFailure, HirRuntimeAssembly,
    InputHandle as RuntimeInputHandle, Publication, SourceProviderContext, SourceProviderManager,
    assemble_hir_runtime_with_items, assemble_hir_runtime_with_items_profiled,
    execute_runtime_value_with_context, link_backend_runtime, render_runtime_error,
};
use aivi_syntax::{Formatter, lex_module, parse_module};
use gtk::{glib, prelude::*};

include!("main_parts/dispatch.rs");
include!("main_parts/workspace.rs");
include!("main_parts/run_model.rs");
include!("main_parts/check_execute.rs");
include!("main_parts/run_prepare.rs");
include!("main_parts/run_hydration.rs");
include!("main_parts/build_tools.rs");

#[cfg(test)]
#[path = "main_parts/tests.rs"]
mod tests;
