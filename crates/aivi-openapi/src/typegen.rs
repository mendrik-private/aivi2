use std::fmt::Write;

use crate::auth::SecuritySchemeKind;
use crate::model::*;
use crate::resolver::ResolvedSpec;

/// The result of generating AIVI types from an OpenAPI spec.
#[derive(Debug, Clone)]
pub struct GeneratedTypeSet {
    /// Complete AIVI source text ready to write to a file.
    pub aivi_source: String,
    /// Names of all generated types (for export/import suggestions).
    pub type_names: Vec<String>,
    /// PascalCase handle type name derived from the spec title.
    pub handle_type_name: String,
}

/// A single generated AIVI type declaration.
#[derive(Debug, Clone)]
pub struct GeneratedType {
    pub name: String,
    pub aivi_decl: String,
}

/// Generate AIVI type declarations from a resolved OpenAPI spec.
pub fn generate_aivi_types(spec: &ResolvedSpec) -> GeneratedTypeSet {
    let mut out = String::new();
    let mut type_names = Vec::new();

    writeln!(
        out,
        "// Auto-generated from OpenAPI spec: {}",
        spec.info.title
    )
    .ok();
    writeln!(out, "// Version: {}", spec.info.version).ok();
    writeln!(
        out,
        "// Do not edit manually — regenerate with: aivi openapi-gen"
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "hoist").ok();
    writeln!(out).ok();

    let auth_variants = generate_auth_variants(spec);
    if !auth_variants.is_empty() {
        writeln!(out, "type ApiAuth =").ok();
        for variant in &auth_variants {
            writeln!(out, "  | {variant}").ok();
        }
        writeln!(out).ok();
        type_names.push("ApiAuth".to_string());
    }

    writeln!(out, "type ApiError =").ok();
    writeln!(out, "  | ApiTimeout").ok();
    writeln!(out, "  | ApiDecodeFailure Text").ok();
    writeln!(out, "  | ApiRequestFailure Text").ok();
    writeln!(out, "  | ApiUnauthorized").ok();
    writeln!(out, "  | ApiNotFound").ok();
    writeln!(out, "  | ApiServerError Text").ok();
    writeln!(out).ok();
    for name in &[
        "ApiError",
        "ApiTimeout",
        "ApiDecodeFailure",
        "ApiRequestFailure",
        "ApiUnauthorized",
        "ApiNotFound",
        "ApiServerError",
    ] {
        type_names.push((*name).to_string());
    }

    for (name, schema) in &spec.components.schemas {
        let pascal = to_pascal_case(name);
        if let Some(decl) = schema_to_aivi_type(&pascal, schema, &spec.components.schemas) {
            writeln!(out, "{decl}").ok();
            writeln!(out).ok();
            type_names.push(pascal);
        }
    }

    let handle_type_name = to_pascal_case(&spec.info.title.replace([' ', '-', '_'], ""));
    writeln!(out, "type {handle_type_name} = Unit").ok();
    writeln!(out).ok();
    type_names.push(handle_type_name.clone());

    write!(out, "export (").ok();
    write!(out, "{}", type_names.join(", ")).ok();
    writeln!(out, ")").ok();

    GeneratedTypeSet {
        aivi_source: out,
        type_names,
        handle_type_name,
    }
}

fn generate_auth_variants(spec: &ResolvedSpec) -> Vec<String> {
    let mut variants = Vec::new();
    for scheme in spec.components.security_schemes.values() {
        let kind = SecuritySchemeKind::from_scheme(scheme);
        let variant = match &kind {
            SecuritySchemeKind::BearerToken => "BearerToken Text".to_string(),
            SecuritySchemeKind::BasicAuth => "BasicAuth Text Text".to_string(),
            SecuritySchemeKind::ApiKeyHeader { .. } => "ApiKey Text".to_string(),
            SecuritySchemeKind::ApiKeyQuery { .. } => "ApiKeyQuery Text".to_string(),
            SecuritySchemeKind::OAuth2 => "OAuth2 Text".to_string(),
            SecuritySchemeKind::Unknown => continue,
        };
        if !variants.contains(&variant) {
            variants.push(variant);
        }
    }
    if variants.is_empty() {
        variants.push("BearerToken Text".to_string());
        variants.push("BasicAuth Text Text".to_string());
        variants.push("ApiKey Text".to_string());
    }
    variants
}

fn schema_to_aivi_type(
    name: &str,
    schema: &Schema,
    components: &indexmap::IndexMap<String, Schema>,
) -> Option<String> {
    // String enum → sum type
    if !schema.enum_values.is_empty() {
        let variants: Vec<String> = schema
            .enum_values
            .iter()
            .filter_map(|v| v.as_str().map(|s| format!("  | {}", to_pascal_case(s))))
            .collect();
        if !variants.is_empty() {
            return Some(format!("type {name} =\n{}", variants.join("\n")));
        }
    }

    // oneOf / anyOf → sum type
    if !schema.one_of.is_empty() || !schema.any_of.is_empty() {
        let schemas = if !schema.one_of.is_empty() {
            &schema.one_of
        } else {
            &schema.any_of
        };
        let discriminator = schema.discriminator.as_ref();
        let variants: Vec<String> = schemas
            .iter()
            .enumerate()
            .map(|(i, sor)| {
                let variant_name = discriminator
                    .and_then(|d| {
                        d.mapping
                            .iter()
                            .find(|(_, v)| v.contains(ref_last_segment(sor)))
                    })
                    .map(|(k, _)| to_pascal_case(k))
                    .unwrap_or_else(|| match sor {
                        SchemaOrRef::Ref(r) => {
                            to_pascal_case(r.ref_path.split('/').next_back().unwrap_or(""))
                        }
                        SchemaOrRef::Schema(s) => s
                            .title
                            .as_deref()
                            .map(to_pascal_case)
                            .unwrap_or_else(|| format!("Variant{i}")),
                    });
                match sor {
                    SchemaOrRef::Ref(r) => {
                        let ref_name =
                            to_pascal_case(r.ref_path.split('/').next_back().unwrap_or(""));
                        format!("  | {variant_name} {ref_name}")
                    }
                    SchemaOrRef::Schema(_) => format!("  | {variant_name}"),
                }
            })
            .collect();
        return Some(format!("type {name} =\n{}", variants.join("\n")));
    }

    // allOf → merged record
    if !schema.all_of.is_empty() {
        let mut all_props = indexmap::IndexMap::new();
        let mut all_required = Vec::new();
        for sor in &schema.all_of {
            let s = match sor {
                SchemaOrRef::Schema(s) => *s.clone(),
                SchemaOrRef::Ref(r) => {
                    let ref_name = r.ref_path.split('/').next_back().unwrap_or("");
                    components.get(ref_name).cloned().unwrap_or_default()
                }
            };
            for (k, v) in s.properties {
                all_props.insert(k, v);
            }
            all_required.extend(s.required);
        }
        let fields: Vec<String> = all_props
            .iter()
            .map(|(fname, fschema)| {
                let required = all_required.contains(fname);
                let field_type = schema_to_aivi_type_expr(fschema, components);
                let field_type = if required {
                    field_type
                } else {
                    format!("Option {field_type}")
                };
                format!("    {}: {}", to_camel_case(fname), field_type)
            })
            .collect();
        if !fields.is_empty() {
            return Some(format!("type {name} = {{\n{}\n}}", fields.join(",\n")));
        }
    }

    // Object with properties → record type
    if schema.properties.is_empty() {
        if matches!(schema.schema_type, Some(SchemaType::Object)) || schema.schema_type.is_none() {
            if let Some(AdditionalProperties::Schema(inner)) = &schema.additional_properties {
                let inner_type = schema_to_aivi_type_expr(inner, components);
                return Some(format!("type {name} = Map Text {inner_type}"));
            }
            return None;
        }
        return None;
    }

    let fields: Vec<String> = schema
        .properties
        .iter()
        .map(|(fname, fschema)| {
            let required = schema.required.contains(fname);
            let field_type = schema_to_aivi_type_expr(fschema, components);
            let field_type = if required {
                field_type
            } else {
                format!("Option {field_type}")
            };
            format!("    {}: {}", to_camel_case(fname), field_type)
        })
        .collect();
    Some(format!("type {name} = {{\n{}\n}}", fields.join(",\n")))
}

fn ref_last_segment(sor: &SchemaOrRef) -> &str {
    match sor {
        SchemaOrRef::Ref(r) => r.ref_path.split('/').next_back().unwrap_or(""),
        _ => "",
    }
}

fn schema_to_aivi_type_expr(
    sor: &SchemaOrRef,
    components: &indexmap::IndexMap<String, Schema>,
) -> String {
    match sor {
        SchemaOrRef::Ref(r) => {
            to_pascal_case(r.ref_path.split('/').next_back().unwrap_or("Unknown"))
        }
        SchemaOrRef::Schema(s) => schema_type_expr(s, components),
    }
}

fn schema_type_expr(schema: &Schema, components: &indexmap::IndexMap<String, Schema>) -> String {
    if !schema.enum_values.is_empty() {
        return "Text".to_string();
    }
    match &schema.schema_type {
        Some(SchemaType::String) => match schema.format.as_deref() {
            Some("binary") => "Bytes".to_string(),
            _ => "Text".to_string(),
        },
        Some(SchemaType::Integer) => "Int".to_string(),
        Some(SchemaType::Number) => "Float".to_string(),
        Some(SchemaType::Boolean) => "Bool".to_string(),
        Some(SchemaType::Array) => {
            let element = schema
                .items
                .as_ref()
                .map(|items| schema_to_aivi_type_expr(items, components))
                .unwrap_or_else(|| "Json".to_string());
            format!("List {element}")
        }
        Some(SchemaType::Object) | None => {
            if let Some(AdditionalProperties::Schema(inner)) = &schema.additional_properties {
                let inner_type = schema_to_aivi_type_expr(inner, components);
                format!("Map Text {inner_type}")
            } else if !schema.properties.is_empty() {
                let fields: Vec<String> = schema
                    .properties
                    .iter()
                    .map(|(fname, fschema)| {
                        let ft = schema_to_aivi_type_expr(fschema, components);
                        let ft = if schema.required.contains(fname) {
                            ft
                        } else {
                            format!("Option {ft}")
                        };
                        format!("{}: {}", to_camel_case(fname), ft)
                    })
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            } else {
                "Unit".to_string()
            }
        }
        Some(SchemaType::Null) => "Unit".to_string(),
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|p| !p.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn to_camel_case(s: &str) -> String {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
    }
}
