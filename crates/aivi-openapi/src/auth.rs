use crate::model::SecurityScheme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecuritySchemeKind {
    BearerToken,
    BasicAuth,
    ApiKeyHeader { header_name: String },
    ApiKeyQuery { param_name: String },
    OAuth2,
    Unknown,
}

impl SecuritySchemeKind {
    pub fn from_scheme(scheme: &SecurityScheme) -> Self {
        match scheme.scheme_type.as_str() {
            "http" => match scheme.scheme.as_deref().unwrap_or("").to_lowercase().as_str() {
                "bearer" => Self::BearerToken,
                "basic" => Self::BasicAuth,
                _ => Self::Unknown,
            },
            "apiKey" => match scheme.location.as_deref().unwrap_or("") {
                "header" => Self::ApiKeyHeader {
                    header_name: scheme
                        .name
                        .clone()
                        .unwrap_or_else(|| "X-API-Key".to_string()),
                },
                "query" => Self::ApiKeyQuery {
                    param_name: scheme
                        .name
                        .clone()
                        .unwrap_or_else(|| "apiKey".to_string()),
                },
                _ => Self::Unknown,
            },
            "oauth2" => Self::OAuth2,
            _ => Self::Unknown,
        }
    }

    pub const fn aivi_variant_name(&self) -> &'static str {
        match self {
            Self::BearerToken => "BearerToken",
            Self::BasicAuth => "BasicAuth",
            Self::ApiKeyHeader { .. } => "ApiKey",
            Self::ApiKeyQuery { .. } => "ApiKeyQuery",
            Self::OAuth2 => "OAuth2",
            Self::Unknown => "NoAuth",
        }
    }
}
