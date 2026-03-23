#![forbid(unsafe_code)]

//! Incremental query/database foundation for AIVI tooling.
//!
//! The current layer is intentionally narrow and honest: it tracks source text as durable inputs,
//! memoises parse/HIR results per file revision, preserves source snapshots for span-driven editor
//! features, and avoids inventing parallel frontend structures. Cross-file resolution and typed
//! queries remain future work, but they now have explicit boundaries to build on.

mod db;
mod inputs;
mod queries;

pub use db::RootDatabase;
pub use inputs::SourceFile;
pub use queries::{
    HirModuleResult, ParsedFileResult, all_diagnostics, exported_names, format_file, hir_module,
    parsed_file, symbol_index,
};
