use crate::resolver::{ResolvedOperation, ResolvedSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl OperationMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    /// Whether this operation is read-only (suitable for a reactive `signal`).
    pub const fn is_read_only(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
    Cookie,
}

#[derive(Debug, Clone)]
pub struct OperationInfo {
    pub operation_id: String,
    pub method: OperationMethod,
    pub path: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub deprecated: bool,
}

/// Look up an operation by `operationId` in the resolved spec.
pub fn find_operation<'a>(
    spec: &'a ResolvedSpec,
    operation_id: &str,
) -> Option<(&'a str, &'a ResolvedOperation)> {
    for (path, path_item) in &spec.paths {
        for op in &path_item.operations {
            if op.operation_id == operation_id {
                return Some((path.as_str(), op));
            }
        }
    }
    None
}

/// Collect all operation infos from the spec.
pub fn all_operations(spec: &ResolvedSpec) -> Vec<OperationInfo> {
    let mut ops = Vec::new();
    for (path, path_item) in &spec.paths {
        for op in &path_item.operations {
            ops.push(OperationInfo {
                operation_id: op.operation_id.clone(),
                method: op.method,
                path: path.clone(),
                summary: op.summary.clone(),
                description: op.description.clone(),
                deprecated: op.deprecated,
            });
        }
    }
    ops
}
