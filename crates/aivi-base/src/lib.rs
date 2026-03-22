#![forbid(unsafe_code)]

//! Foundational source and diagnostic infrastructure shared by every AIVI layer.

pub mod diagnostic;
pub mod source;

pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLabel, LabelStyle, Severity};
pub use source::{
    ByteIndex, FileId, LineColumn, LspPosition, LspRange, SourceDatabase, SourceFile, SourceSpan,
    Span, Spanned,
};
