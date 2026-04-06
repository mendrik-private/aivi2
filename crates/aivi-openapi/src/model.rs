use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Top-level OpenAPI 3.x document.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenApiSpec {
    pub openapi: String,
    pub info: Info,
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub paths: IndexMap<String, PathItem>,
    #[serde(default)]
    pub components: Option<Components>,
    #[serde(default)]
    pub security: Vec<SecurityRequirement>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub webhooks: IndexMap<String, PathItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    pub title: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Server {
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PathItem {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub get: Option<Operation>,
    #[serde(default)]
    pub put: Option<Operation>,
    #[serde(default)]
    pub post: Option<Operation>,
    #[serde(default)]
    pub delete: Option<Operation>,
    #[serde(default)]
    pub patch: Option<Operation>,
    #[serde(default)]
    pub head: Option<Operation>,
    #[serde(default)]
    pub options: Option<Operation>,
    #[serde(default)]
    pub parameters: Vec<ParameterOrRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<ParameterOrRef>,
    #[serde(default)]
    pub request_body: Option<RequestBodyOrRef>,
    #[serde(default)]
    pub responses: IndexMap<String, ResponseOrRef>,
    #[serde(default)]
    pub security: Option<Vec<SecurityRequirement>>,
    #[serde(default)]
    pub deprecated: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ParameterOrRef {
    Ref(Ref),
    Parameter(Parameter),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: ParameterIn,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub deprecated: bool,
    pub schema: Option<SchemaOrRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParameterIn {
    Query,
    Header,
    Path,
    Cookie,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RequestBodyOrRef {
    Ref(Ref),
    RequestBody(RequestBody),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestBody {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    pub content: IndexMap<String, MediaType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaType {
    pub schema: Option<SchemaOrRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponseOrRef {
    Ref(Ref),
    Response(Response),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Response {
    pub description: String,
    #[serde(default)]
    pub content: IndexMap<String, MediaType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Components {
    #[serde(default)]
    pub schemas: IndexMap<String, SchemaOrRef>,
    #[serde(default)]
    pub parameters: IndexMap<String, ParameterOrRef>,
    #[serde(default)]
    pub responses: IndexMap<String, ResponseOrRef>,
    #[serde(default)]
    pub request_bodies: IndexMap<String, RequestBodyOrRef>,
    #[serde(default)]
    pub security_schemes: IndexMap<String, SecuritySchemeOrRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SchemaOrRef {
    Ref(Ref),
    Schema(Box<Schema>),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    #[serde(rename = "type", default)]
    pub schema_type: Option<SchemaType>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: IndexMap<String, SchemaOrRef>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub additional_properties: Option<AdditionalProperties>,
    #[serde(default)]
    pub items: Option<Box<SchemaOrRef>>,
    #[serde(default, rename = "enum")]
    pub enum_values: Vec<serde_json::Value>,
    #[serde(default)]
    pub all_of: Vec<SchemaOrRef>,
    #[serde(default)]
    pub one_of: Vec<SchemaOrRef>,
    #[serde(default)]
    pub any_of: Vec<SchemaOrRef>,
    #[serde(default)]
    pub discriminator: Option<Discriminator>,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    #[serde(default)]
    pub min_length: Option<usize>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub write_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaType {
    Object,
    Array,
    String,
    Integer,
    Number,
    Boolean,
    Null,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    Bool(bool),
    Schema(Box<SchemaOrRef>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discriminator {
    pub property_name: String,
    #[serde(default)]
    pub mapping: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ref {
    #[serde(rename = "$ref")]
    pub ref_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SecuritySchemeOrRef {
    Ref(Ref),
    SecurityScheme(SecurityScheme),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScheme {
    #[serde(rename = "type")]
    pub scheme_type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "in", default)]
    pub location: Option<String>,
    #[serde(default)]
    pub scheme: Option<String>,
    #[serde(default)]
    pub bearer_format: Option<String>,
    #[serde(default)]
    pub flows: Option<OAuthFlows>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthFlows {
    #[serde(default)]
    pub implicit: Option<OAuthFlow>,
    #[serde(default)]
    pub password: Option<OAuthFlow>,
    #[serde(default)]
    pub client_credentials: Option<OAuthFlow>,
    #[serde(default)]
    pub authorization_code: Option<OAuthFlow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthFlow {
    #[serde(default)]
    pub authorization_url: Option<String>,
    #[serde(default)]
    pub token_url: Option<String>,
    #[serde(default)]
    pub scopes: HashMap<String, String>,
}

pub type SecurityRequirement = HashMap<String, Vec<String>>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tag {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}
