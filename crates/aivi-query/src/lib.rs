#![forbid(unsafe_code)]

//! Incremental query/database foundation for AIVI tooling.
//!
//! The current layer is intentionally narrow and honest: it tracks source text as durable inputs,
//! memoises parse/HIR results per file revision, preserves source snapshots for span-driven editor
//! features, keeps multi-file workspace import resolution explicit through deterministic
//! file-to-module mapping plus registered reverse dependencies, and now exposes first typed
//! backend-unit queries for whole-program runtime lowering, runtime fragments, and stable
//! fingerprints that later JIT/cache layers can key on.

mod db;
mod entry;
mod inputs;
mod manifest;
mod queries;
mod workspace;

pub use db::{QueryCacheStats, RootDatabase};
pub use entry::{
    EntrypointOrigin, EntrypointResolutionError, ResolvedEntrypoint, resolve_v1_entrypoint,
};
pub use inputs::SourceFile;
pub use manifest::{AiviManifest, AppConfig, RunConfig, WorkspaceConfig, parse_manifest};
pub use queries::{
    BackendUnitError, HirModuleResult, ParsedFileResult, RuntimeFragmentBackendUnit,
    RuntimeFragmentFingerprint, StableFingerprint, WholeProgramBackendUnit,
    WholeProgramFingerprint, WorkspaceHirModule, all_diagnostics, exported_names, format_file,
    hir_module, parsed_file, reachable_workspace_hir_modules, resolve_module_file,
    runtime_fragment_backend_fingerprint, runtime_fragment_backend_unit, symbol_index,
    whole_program_backend_fingerprint, whole_program_backend_fingerprint_with_items,
    whole_program_backend_unit, whole_program_backend_unit_with_items,
};
pub use workspace::{discover_workspace_root, discover_workspace_root_from_directory};
