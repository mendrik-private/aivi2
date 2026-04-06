use std::path::Path;

use crate::diagnostics::SpecDiagnostic;
use crate::model::OpenApiSpec;

/// Parse a YAML or JSON OpenAPI spec from a file path.
pub fn parse_spec(path: &Path) -> Result<OpenApiSpec, SpecDiagnostic> {
    let content = std::fs::read_to_string(path).map_err(|e| SpecDiagnostic::io_error(path, &e))?;
    parse_spec_str(&content, path)
}

/// Parse a YAML or JSON OpenAPI spec from a string, using `path` for diagnostics and extension detection.
pub fn parse_spec_str(content: &str, path: &Path) -> Result<OpenApiSpec, SpecDiagnostic> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let spec: OpenApiSpec = if extension == "json" {
        serde_json::from_str(content)
            .map_err(|e| SpecDiagnostic::parse_error(path, &e.to_string()))?
    } else {
        serde_yaml::from_str(content)
            .map_err(|e| SpecDiagnostic::parse_error(path, &e.to_string()))?
    };
    validate_version(&spec, path)?;
    Ok(spec)
}

fn validate_version(spec: &OpenApiSpec, path: &Path) -> Result<(), SpecDiagnostic> {
    if !spec.openapi.starts_with("3.") {
        return Err(SpecDiagnostic::unsupported_version(path, &spec.openapi));
    }
    Ok(())
}
