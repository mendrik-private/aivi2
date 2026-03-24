#![forbid(unsafe_code)]

//! Incremental query/database foundation for AIVI tooling.
//!
//! The current layer is intentionally narrow and honest: it tracks source text as durable inputs,
//! memoises parse/HIR results per file revision, preserves source snapshots for span-driven editor
//! features, and keeps multi-file workspace import resolution explicit through deterministic
//! file-to-module mapping plus registered reverse dependencies. Typed queries remain future work.

mod db;
mod inputs;
mod queries;
mod workspace;

pub use db::RootDatabase;
pub use inputs::SourceFile;
pub use queries::{
    HirModuleResult, ParsedFileResult, all_diagnostics, exported_names, format_file, hir_module,
    parsed_file, symbol_index,
};
