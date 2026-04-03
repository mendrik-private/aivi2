#![forbid(unsafe_code)]

//! Foundational source and diagnostic infrastructure shared by every AIVI layer.

pub mod arena;
pub mod diagnostic;
pub mod errors;
pub mod render;
pub mod source;

pub use arena::{Arena, ArenaId, ArenaOverflow};
pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLabel, LabelStyle, Severity};
pub use errors::ErrorCollection;
pub use render::{ColorMode, DiagnosticRenderer};
pub use source::{
    ByteIndex, FileId, LineColumn, LspPosition, LspRange, SourceDatabase, SourceFile, SourceSpan,
    Span, Spanned,
};
