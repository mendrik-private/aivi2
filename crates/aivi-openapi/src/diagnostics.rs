use std::path::Path;

/// Diagnostic produced during OpenAPI spec parsing or resolution.
#[derive(Debug, Clone)]
pub struct SpecDiagnostic {
    pub kind: SpecDiagnosticKind,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecDiagnosticKind {
    IoError,
    ParseError,
    ResolutionError,
    UnsupportedVersion,
    UnknownOperation,
    ValidationError,
}

impl SpecDiagnostic {
    pub fn io_error(path: &Path, error: &std::io::Error) -> Self {
        Self {
            kind: SpecDiagnosticKind::IoError,
            message: format!("cannot read spec file `{}`: {error}", path.display()),
            path: Some(path.display().to_string()),
        }
    }

    pub fn parse_error(path: &Path, error: &str) -> Self {
        Self {
            kind: SpecDiagnosticKind::ParseError,
            message: format!("failed to parse OpenAPI spec `{}`: {error}", path.display()),
            path: Some(path.display().to_string()),
        }
    }

    pub fn unsupported_version(path: &Path, version: &str) -> Self {
        Self {
            kind: SpecDiagnosticKind::UnsupportedVersion,
            message: format!(
                "unsupported OpenAPI version `{version}` in `{}`; only OpenAPI 3.x is supported",
                path.display()
            ),
            path: Some(path.display().to_string()),
        }
    }

    pub fn unknown_operation(spec_path: &str, operation_id: &str) -> Self {
        Self {
            kind: SpecDiagnosticKind::UnknownOperation,
            message: format!(
                "operation `{operation_id}` not found in OpenAPI spec `{spec_path}`"
            ),
            path: Some(spec_path.to_string()),
        }
    }

    pub fn resolution_error(path: &Path, message: &str) -> Self {
        Self {
            kind: SpecDiagnosticKind::ResolutionError,
            message: format!("spec resolution error in `{}`: {message}", path.display()),
            path: Some(path.display().to_string()),
        }
    }
}

impl std::fmt::Display for SpecDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SpecDiagnostic {}
