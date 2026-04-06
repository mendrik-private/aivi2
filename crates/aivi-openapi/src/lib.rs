pub mod auth;
pub mod diagnostics;
pub mod model;
pub mod operations;
pub mod parser;
pub mod resolver;
pub mod typegen;

pub use auth::SecuritySchemeKind;
pub use diagnostics::SpecDiagnostic;
pub use model::*;
pub use operations::{OperationInfo, OperationMethod, ParameterLocation};
pub use parser::parse_spec;
pub use resolver::resolve_spec;
pub use typegen::{GeneratedType, GeneratedTypeSet, generate_aivi_types};

use std::path::Path;

/// Parse and resolve a spec, then find a single operation by its operationId.
///
/// Returns `None` when the spec cannot be read/parsed, or the operation is absent.
pub fn parse_spec_and_find_operation(spec_path: &Path, operation_id: &str) -> Option<OperationInfo> {
    let spec = parse_spec(spec_path).ok()?;
    let resolved = resolve_spec(spec, spec_path).ok()?;
    let (path, op) = operations::find_operation(&resolved, operation_id)?;
    Some(OperationInfo {
        operation_id: op.operation_id.clone(),
        method: op.method,
        path: path.to_string(),
        summary: op.summary.clone(),
        description: op.description.clone(),
        deprecated: op.deprecated,
    })
}
