use std::path::Path;

use crate::diagnostics::SpecDiagnostic;
use crate::model::*;
use crate::operations::OperationMethod;

/// A spec with all `$ref`s resolved into the inline model.
#[derive(Debug, Clone)]
pub struct ResolvedSpec {
    pub info: Info,
    pub servers: Vec<Server>,
    pub paths: indexmap::IndexMap<String, ResolvedPathItem>,
    pub components: ResolvedComponents,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedPathItem {
    pub path: String,
    pub operations: Vec<ResolvedOperation>,
    pub shared_parameters: Vec<ResolvedParameter>,
}

#[derive(Debug, Clone)]
pub struct ResolvedOperation {
    pub method: OperationMethod,
    pub operation_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub parameters: Vec<ResolvedParameter>,
    pub request_body: Option<ResolvedRequestBody>,
    pub response_schema: Option<Schema>,
    pub response_description: String,
    pub security: Option<Vec<SecurityRequirement>>,
    pub deprecated: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedParameter {
    pub name: String,
    pub location: ParameterIn,
    pub required: bool,
    pub schema: Option<Schema>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRequestBody {
    pub required: bool,
    pub schema: Option<Schema>,
    pub content_type: String,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedComponents {
    pub schemas: indexmap::IndexMap<String, Schema>,
    pub security_schemes: indexmap::IndexMap<String, SecurityScheme>,
}

/// Resolve all `$ref`s and derive `operationId`s for anonymous operations.
pub fn resolve_spec(spec: OpenApiSpec, _path: &Path) -> Result<ResolvedSpec, Vec<SpecDiagnostic>> {
    let mut diagnostics = Vec::new();
    let components = resolve_components(spec.components.as_ref());
    let mut paths = indexmap::IndexMap::new();

    for (path_str, path_item) in &spec.paths {
        let resolved = resolve_path_item(path_str, path_item, &components, &mut diagnostics);
        paths.insert(path_str.clone(), resolved);
    }

    for (name, path_item) in &spec.webhooks {
        let key = format!("webhooks/{name}");
        let resolved = resolve_path_item(&key, path_item, &components, &mut diagnostics);
        paths.insert(key, resolved);
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(ResolvedSpec {
        info: spec.info,
        servers: spec.servers,
        paths,
        components,
        tags: spec.tags,
    })
}

fn resolve_components(components: Option<&Components>) -> ResolvedComponents {
    let Some(comp) = components else {
        return ResolvedComponents::default();
    };
    let mut schemas = indexmap::IndexMap::new();
    for (name, schema_or_ref) in &comp.schemas {
        if let SchemaOrRef::Schema(schema) = schema_or_ref {
            schemas.insert(name.clone(), *schema.clone());
        }
    }
    let mut security_schemes = indexmap::IndexMap::new();
    for (name, scheme_or_ref) in &comp.security_schemes {
        if let SecuritySchemeOrRef::SecurityScheme(scheme) = scheme_or_ref {
            security_schemes.insert(name.clone(), scheme.clone());
        }
    }
    ResolvedComponents {
        schemas,
        security_schemes,
    }
}

fn resolve_path_item(
    path_str: &str,
    item: &PathItem,
    components: &ResolvedComponents,
    diagnostics: &mut Vec<SpecDiagnostic>,
) -> ResolvedPathItem {
    let shared_params = resolve_parameters(&item.parameters, components);
    let methods = [
        (OperationMethod::Get, item.get.as_ref()),
        (OperationMethod::Put, item.put.as_ref()),
        (OperationMethod::Post, item.post.as_ref()),
        (OperationMethod::Delete, item.delete.as_ref()),
        (OperationMethod::Patch, item.patch.as_ref()),
        (OperationMethod::Head, item.head.as_ref()),
        (OperationMethod::Options, item.options.as_ref()),
    ];
    let mut operations = Vec::new();
    for (method, op_opt) in methods {
        let Some(op) = op_opt else { continue };
        let operation_id = op
            .operation_id
            .clone()
            .unwrap_or_else(|| derive_operation_id(method, path_str));
        let mut parameters = shared_params.clone();
        parameters.extend(resolve_parameters(&op.parameters, components));
        let request_body = op
            .request_body
            .as_ref()
            .and_then(|rb| resolve_request_body(rb, components));
        let (response_schema, response_description) =
            resolve_success_response(&op.responses, components);
        let _ = diagnostics;
        operations.push(ResolvedOperation {
            method,
            operation_id,
            summary: op.summary.clone(),
            description: op.description.clone(),
            tags: op.tags.clone(),
            parameters,
            request_body,
            response_schema,
            response_description: response_description.unwrap_or_default(),
            security: op.security.clone(),
            deprecated: op.deprecated,
        });
    }
    ResolvedPathItem {
        path: path_str.to_string(),
        operations,
        shared_parameters: shared_params,
    }
}

fn derive_operation_id(method: OperationMethod, path: &str) -> String {
    let method_str = method.as_str().to_lowercase();
    let path_parts: String = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                capitalize_first(&segment[1..segment.len() - 1])
            } else {
                capitalize_first(segment)
            }
        })
        .collect();
    format!("{method_str}{path_parts}")
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn resolve_parameters(
    params: &[ParameterOrRef],
    components: &ResolvedComponents,
) -> Vec<ResolvedParameter> {
    params
        .iter()
        .filter_map(|p| match p {
            ParameterOrRef::Parameter(param) => Some(ResolvedParameter {
                name: param.name.clone(),
                location: param.location.clone(),
                required: param.required,
                schema: resolve_schema_opt(param.schema.as_ref(), components),
                description: param.description.clone(),
            }),
            ParameterOrRef::Ref(_) => None,
        })
        .collect()
}

fn resolve_request_body(
    rb: &RequestBodyOrRef,
    components: &ResolvedComponents,
) -> Option<ResolvedRequestBody> {
    match rb {
        RequestBodyOrRef::RequestBody(body) => {
            let (content_type, schema) = body
                .content
                .iter()
                .next()
                .map(|(ct, media)| {
                    (
                        ct.clone(),
                        resolve_schema_opt(media.schema.as_ref(), components),
                    )
                })
                .unwrap_or_else(|| ("application/json".to_string(), None));
            Some(ResolvedRequestBody {
                required: body.required,
                schema,
                content_type,
            })
        }
        RequestBodyOrRef::Ref(_) => None,
    }
}

fn resolve_success_response(
    responses: &indexmap::IndexMap<String, ResponseOrRef>,
    components: &ResolvedComponents,
) -> (Option<Schema>, Option<String>) {
    for code in &["200", "201", "204", "2XX", "default"] {
        if let Some(resp) = responses.get(*code) {
            let (schema, desc) = match resp {
                ResponseOrRef::Response(r) => {
                    let schema = r.content.iter().next().and_then(|(_, media)| {
                        resolve_schema_opt(media.schema.as_ref(), components)
                    });
                    (schema, Some(r.description.clone()))
                }
                ResponseOrRef::Ref(_) => (None, None),
            };
            return (schema, desc);
        }
    }
    (None, None)
}

fn resolve_schema_opt(
    schema_or_ref: Option<&SchemaOrRef>,
    components: &ResolvedComponents,
) -> Option<Schema> {
    Some(resolve_schema_or_ref(schema_or_ref?, components))
}

fn resolve_schema_or_ref(sor: &SchemaOrRef, components: &ResolvedComponents) -> Schema {
    match sor {
        SchemaOrRef::Schema(s) => *s.clone(),
        SchemaOrRef::Ref(r) => {
            let name = r.ref_path.split('/').next_back().unwrap_or("");
            components.schemas.get(name).cloned().unwrap_or_default()
        }
    }
}
