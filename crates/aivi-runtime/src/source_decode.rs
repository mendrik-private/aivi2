use std::collections::{BTreeMap, BTreeSet};

use aivi_backend::{
    RuntimeBigInt, RuntimeDecimal, RuntimeFloat, RuntimeRecordField, RuntimeSumValue, RuntimeValue,
};
use aivi_hir::{DecodeProgramStep, SourceDecodeProgram};
use aivi_typing::DecodeExtraFieldPolicy;
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalSourceValue {
    Unit,
    Bool(bool),
    Int(i64),
    Float(RuntimeFloat),
    Text(Box<str>),
    List(Vec<ExternalSourceValue>),
    Record(BTreeMap<Box<str>, ExternalSourceValue>),
    Variant {
        name: Box<str>,
        payload: Option<Box<ExternalSourceValue>>,
    },
}

impl ExternalSourceValue {
    pub fn variant(name: impl Into<Box<str>>) -> Self {
        Self::Variant {
            name: name.into(),
            payload: None,
        }
    }

    pub fn variant_with_payload(name: impl Into<Box<str>>, payload: ExternalSourceValue) -> Self {
        Self::Variant {
            name: name.into(),
            payload: Some(Box::new(payload)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeProgramSupportError {
    UnsupportedScalar { scalar: &'static str },
    UnsupportedDomain { member_name: Box<str> },
}

impl std::fmt::Display for SourceDecodeProgramSupportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScalar { scalar } => {
                write!(
                    f,
                    "runtime source decoding does not execute `{scalar}` payloads yet"
                )
            }
            Self::UnsupportedDomain { member_name } => write!(
                f,
                "runtime source decoding does not execute domain surface `{member_name}` yet"
            ),
        }
    }
}

impl std::error::Error for SourceDecodeProgramSupportError {}

pub fn validate_supported_program(
    program: &SourceDecodeProgram,
) -> Result<(), SourceDecodeProgramSupportError> {
    for step in program.steps() {
        match step {
            DecodeProgramStep::Scalar { scalar } => match scalar {
                aivi_typing::PrimitiveType::Unit
                | aivi_typing::PrimitiveType::Bool
                | aivi_typing::PrimitiveType::Int
                | aivi_typing::PrimitiveType::Float
                | aivi_typing::PrimitiveType::Decimal
                | aivi_typing::PrimitiveType::BigInt
                | aivi_typing::PrimitiveType::Bytes
                | aivi_typing::PrimitiveType::Text => {}
            },
            DecodeProgramStep::Domain { surface, .. } => {
                return Err(SourceDecodeProgramSupportError::UnsupportedDomain {
                    member_name: surface.member_name.clone(),
                });
            }
            DecodeProgramStep::Tuple { .. }
            | DecodeProgramStep::Record { .. }
            | DecodeProgramStep::Sum { .. }
            | DecodeProgramStep::List { .. }
            | DecodeProgramStep::Option { .. }
            | DecodeProgramStep::Result { .. }
            | DecodeProgramStep::Validation { .. } => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeError {
    InvalidJson {
        detail: Box<str>,
    },
    UnsupportedNumber {
        value: Box<str>,
    },
    InvalidScalarLiteral {
        scalar: &'static str,
        value: Box<str>,
    },
    InvalidBytesElementKind {
        index: usize,
        found: &'static str,
    },
    InvalidByteValue {
        index: usize,
        value: i64,
    },
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    InvalidTupleLength {
        expected: usize,
        found: usize,
    },
    MissingField {
        field: Box<str>,
    },
    UnexpectedFields {
        fields: Box<[Box<str>]>,
    },
    UnknownVariant {
        found: Box<str>,
        expected: Box<[Box<str>]>,
    },
    MissingVariantPayload {
        variant: Box<str>,
    },
    UnexpectedVariantPayload {
        variant: Box<str>,
    },
    UnsupportedProgram(SourceDecodeProgramSupportError),
}

impl std::fmt::Display for SourceDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson { detail } => write!(f, "invalid JSON source payload: {detail}"),
            Self::UnsupportedNumber { value } => {
                write!(
                    f,
                    "source payload number `{value}` does not fit the current Int/Float runtime slice"
                )
            }
            Self::InvalidScalarLiteral { scalar, value } => {
                write!(
                    f,
                    "source payload `{value}` is not a valid {scalar} literal for the current runtime decode contract"
                )
            }
            Self::InvalidBytesElementKind { index, found } => {
                write!(
                    f,
                    "source payload byte array element {index} must be an integer octet, found {found}"
                )
            }
            Self::InvalidByteValue { index, value } => {
                write!(
                    f,
                    "source payload byte array element {index} must be between 0 and 255, found {value}"
                )
            }
            Self::TypeMismatch { expected, found } => {
                write!(f, "source payload expected {expected}, found {found}")
            }
            Self::InvalidTupleLength { expected, found } => {
                write!(
                    f,
                    "source payload expected tuple/list length {expected}, found {found}"
                )
            }
            Self::MissingField { field } => {
                write!(
                    f,
                    "source payload record is missing required field `{field}`"
                )
            }
            Self::UnexpectedFields { fields } => {
                write!(
                    f,
                    "source payload record contains unexpected fields {:?}",
                    fields
                )
            }
            Self::UnknownVariant { found, expected } => {
                write!(
                    f,
                    "source payload variant `{found}` is not one of {:?}",
                    expected
                )
            }
            Self::MissingVariantPayload { variant } => {
                write!(
                    f,
                    "source payload variant `{variant}` is missing its payload"
                )
            }
            Self::UnexpectedVariantPayload { variant } => {
                write!(
                    f,
                    "source payload variant `{variant}` must not carry a payload"
                )
            }
            Self::UnsupportedProgram(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SourceDecodeError {}

pub fn parse_json_text(text: &str) -> Result<ExternalSourceValue, SourceDecodeError> {
    let value = serde_json::from_str::<JsonValue>(text).map_err(|error| {
        SourceDecodeError::InvalidJson {
            detail: error.to_string().into_boxed_str(),
        }
    })?;
    external_from_json(value)
}

/// Maximum recursion depth for `decode_step`. JSON structures nested beyond this limit are
/// rejected with an error rather than recursed into, preventing stack overflow on adversarial or
/// deeply nested input.
const MAX_DECODE_DEPTH: usize = 512;

pub fn decode_external(
    program: &SourceDecodeProgram,
    value: &ExternalSourceValue,
) -> Result<RuntimeValue, SourceDecodeError> {
    validate_supported_program(program).map_err(SourceDecodeError::UnsupportedProgram)?;
    decode_step(program, program.root_step(), value, 0)
}

pub fn encode_runtime_json(value: &RuntimeValue) -> Result<String, Box<str>> {
    let encoded = runtime_to_json(value)?;
    serde_json::to_string(&encoded).map_err(|error| error.to_string().into_boxed_str())
}

fn external_from_json(value: JsonValue) -> Result<ExternalSourceValue, SourceDecodeError> {
    match value {
        JsonValue::Null => Ok(ExternalSourceValue::Unit),
        JsonValue::Bool(value) => Ok(ExternalSourceValue::Bool(value)),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(ExternalSourceValue::Int(value))
            } else if let Some(value) = number.as_f64().and_then(RuntimeFloat::new) {
                // NOTE: JSON numbers that do not fit in i64 are decoded as f64.
                // Integers larger than 2^53 will be silently truncated because f64 cannot
                // represent all integers in that range exactly. A future improvement would
                // be to detect such values and use a big-integer or decimal type instead.
                Ok(ExternalSourceValue::Float(value))
            } else {
                Err(SourceDecodeError::UnsupportedNumber {
                    value: number.to_string().into_boxed_str(),
                })
            }
        }
        JsonValue::String(value) => Ok(ExternalSourceValue::Text(value.into_boxed_str())),
        JsonValue::Array(values) => values
            .into_iter()
            .map(external_from_json)
            .collect::<Result<Vec<_>, _>>()
            .map(ExternalSourceValue::List),
        JsonValue::Object(fields) => {
            if let Some(JsonValue::String(tag)) = fields.get("tag")
                && fields.len() <= 2
            {
                let payload = fields
                    .get("payload")
                    .cloned()
                    .map(external_from_json)
                    .transpose()?
                    .map(Box::new);
                return Ok(ExternalSourceValue::Variant {
                    name: tag.clone().into_boxed_str(),
                    payload,
                });
            }
            let mut record = BTreeMap::new();
            for (name, value) in fields {
                record.insert(name.into_boxed_str(), external_from_json(value)?);
            }
            Ok(ExternalSourceValue::Record(record))
        }
    }
}

fn runtime_to_json(value: &RuntimeValue) -> Result<JsonValue, Box<str>> {
    match value {
        RuntimeValue::Unit => Ok(JsonValue::Null),
        RuntimeValue::Bool(value) => Ok(JsonValue::Bool(*value)),
        RuntimeValue::Int(value) => Ok(JsonValue::Number((*value).into())),
        RuntimeValue::Float(value) => serde_json::Number::from_f64(value.to_f64())
            .map(JsonValue::Number)
            .ok_or_else(|| "runtime JSON encoding rejected a non-finite Float value".into()),
        RuntimeValue::Decimal(value) => Ok(JsonValue::String(value.to_string())),
        RuntimeValue::BigInt(value) => Ok(JsonValue::String(value.to_string())),
        RuntimeValue::Text(value) => Ok(JsonValue::String(value.as_ref().to_owned())),
        RuntimeValue::Tuple(values) | RuntimeValue::List(values) | RuntimeValue::Set(values) => {
            values
                .iter()
                .map(runtime_to_json)
                .collect::<Result<Vec<_>, _>>()
                .map(JsonValue::Array)
        }
        RuntimeValue::Record(fields) => {
            let mut object = serde_json::Map::new();
            for field in fields {
                object.insert(
                    field.label.as_ref().to_owned(),
                    runtime_to_json(&field.value)?,
                );
            }
            Ok(JsonValue::Object(object))
        }
        RuntimeValue::Sum(value) => {
            let mut object = serde_json::Map::new();
            object.insert(
                "tag".into(),
                JsonValue::String(value.variant_name.as_ref().to_owned()),
            );
            match value.fields.as_slice() {
                [] => {}
                [field] => {
                    object.insert("payload".into(), runtime_to_json(field)?);
                }
                fields => {
                    object.insert(
                        "payload".into(),
                        JsonValue::Array(
                            fields
                                .iter()
                                .map(runtime_to_json)
                                .collect::<Result<Vec<_>, _>>()?,
                        ),
                    );
                }
            }
            Ok(JsonValue::Object(object))
        }
        RuntimeValue::OptionNone => Ok(JsonValue::Object(serde_json::Map::from_iter([(
            "tag".into(),
            JsonValue::String("None".into()),
        )]))),
        RuntimeValue::OptionSome(value) => tagged_runtime_json("Some", Some(value)),
        RuntimeValue::ResultOk(value) => tagged_runtime_json("Ok", Some(value)),
        RuntimeValue::ResultErr(value) => tagged_runtime_json("Err", Some(value)),
        RuntimeValue::ValidationValid(value) => tagged_runtime_json("Valid", Some(value)),
        RuntimeValue::ValidationInvalid(value) => tagged_runtime_json("Invalid", Some(value)),
        RuntimeValue::Signal(value) => runtime_to_json(value),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            Ok(JsonValue::String(format!("{raw}{suffix}")))
        }
        RuntimeValue::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries {
                let Some(key) = key.as_text() else {
                    return Err("runtime JSON encoding requires Text map keys".into());
                };
                object.insert(key.to_owned(), runtime_to_json(value)?);
            }
            Ok(JsonValue::Object(object))
        }
        RuntimeValue::Callable(_) => {
            Err("runtime JSON encoding does not support callable values".into())
        }
        RuntimeValue::Bytes(bytes) => Ok(JsonValue::Array(
            bytes
                .iter()
                .map(|byte| JsonValue::Number(serde_json::Number::from(*byte)))
                .collect(),
        )),
        RuntimeValue::Task(_) | RuntimeValue::DbTask(_) => {
            Err("runtime JSON encoding does not support Task values".into())
        }
    }
}

fn tagged_runtime_json(tag: &str, payload: Option<&RuntimeValue>) -> Result<JsonValue, Box<str>> {
    let mut object = serde_json::Map::new();
    object.insert("tag".into(), JsonValue::String(tag.to_owned()));
    if let Some(payload) = payload {
        object.insert("payload".into(), runtime_to_json(payload)?);
    }
    Ok(JsonValue::Object(object))
}

fn decode_literal_scalar<T>(
    value: &ExternalSourceValue,
    scalar: &'static str,
    expected: &'static str,
    parse: impl FnOnce(&str) -> Option<T>,
    build: impl FnOnce(T) -> RuntimeValue,
) -> Result<RuntimeValue, SourceDecodeError> {
    let ExternalSourceValue::Text(value) = value else {
        return Err(type_mismatch(expected, value));
    };
    parse(value.as_ref())
        .map(build)
        .ok_or_else(|| SourceDecodeError::InvalidScalarLiteral {
            scalar,
            value: value.clone(),
        })
}

fn decode_bytes_scalar(value: &ExternalSourceValue) -> Result<RuntimeValue, SourceDecodeError> {
    let ExternalSourceValue::List(values) = value else {
        return Err(type_mismatch("byte array", value));
    };
    let mut bytes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let ExternalSourceValue::Int(value) = value else {
            return Err(SourceDecodeError::InvalidBytesElementKind {
                index,
                found: value_kind(value),
            });
        };
        let Ok(byte) = u8::try_from(*value) else {
            return Err(SourceDecodeError::InvalidByteValue {
                index,
                value: *value,
            });
        };
        bytes.push(byte);
    }
    Ok(RuntimeValue::Bytes(bytes.into_boxed_slice()))
}

fn decode_step(
    program: &SourceDecodeProgram,
    step: &DecodeProgramStep,
    value: &ExternalSourceValue,
    depth: usize,
) -> Result<RuntimeValue, SourceDecodeError> {
    if depth > MAX_DECODE_DEPTH {
        return Err(SourceDecodeError::TypeMismatch {
            expected: "value within nesting depth limit",
            found: "structure nested too deeply (> 512 levels)",
        });
    }
    match step {
        DecodeProgramStep::Scalar { scalar } => match scalar {
            aivi_typing::PrimitiveType::Unit => match value {
                ExternalSourceValue::Unit => Ok(RuntimeValue::Unit),
                other => Err(type_mismatch("unit", other)),
            },
            aivi_typing::PrimitiveType::Bool => match value {
                ExternalSourceValue::Bool(value) => Ok(RuntimeValue::Bool(*value)),
                other => Err(type_mismatch("bool", other)),
            },
            aivi_typing::PrimitiveType::Int => match value {
                ExternalSourceValue::Int(value) => Ok(RuntimeValue::Int(*value)),
                other => Err(type_mismatch("integer", other)),
            },
            aivi_typing::PrimitiveType::Float => match value {
                ExternalSourceValue::Int(value) => Ok(RuntimeValue::Float(
                    RuntimeFloat::new(*value as f64)
                        .expect("all i64 values should map to finite f64 values"),
                )),
                ExternalSourceValue::Float(value) => Ok(RuntimeValue::Float(*value)),
                other => Err(type_mismatch("float", other)),
            },
            aivi_typing::PrimitiveType::Text => match value {
                ExternalSourceValue::Text(value) => Ok(RuntimeValue::Text(value.clone())),
                other => Err(type_mismatch("text", other)),
            },
            aivi_typing::PrimitiveType::Decimal => decode_literal_scalar(
                value,
                "Decimal",
                "decimal literal string",
                RuntimeDecimal::parse_literal,
                RuntimeValue::Decimal,
            ),
            aivi_typing::PrimitiveType::BigInt => decode_literal_scalar(
                value,
                "BigInt",
                "bigint literal string",
                RuntimeBigInt::parse_literal,
                RuntimeValue::BigInt,
            ),
            aivi_typing::PrimitiveType::Bytes => decode_bytes_scalar(value),
        },
        DecodeProgramStep::Tuple { elements } => {
            let ExternalSourceValue::List(values) = value else {
                return Err(type_mismatch("list/tuple", value));
            };
            if values.len() != elements.len() {
                return Err(SourceDecodeError::InvalidTupleLength {
                    expected: elements.len(),
                    found: values.len(),
                });
            }
            let decoded = elements
                .iter()
                .zip(values.iter())
                .map(|(element, value)| {
                    decode_step(program, program.step(*element), value, depth + 1)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RuntimeValue::Tuple(decoded))
        }
        DecodeProgramStep::Record {
            fields,
            extra_fields,
        } => {
            let ExternalSourceValue::Record(values) = value else {
                return Err(type_mismatch("record/object", value));
            };
            let mut decoded = Vec::with_capacity(fields.len());
            for field in fields {
                let Some(value) = values.get(field.name.as_str()) else {
                    return Err(SourceDecodeError::MissingField {
                        field: field.name.as_str().into(),
                    });
                };
                decoded.push(RuntimeRecordField {
                    label: field.name.as_str().into(),
                    value: decode_step(program, program.step(field.step), value, depth + 1)?,
                });
            }
            if *extra_fields == DecodeExtraFieldPolicy::Reject {
                let expected = fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<BTreeSet<_>>();
                let extras = values
                    .keys()
                    .filter(|field| !expected.contains(field.as_ref()))
                    .cloned()
                    .collect::<Vec<_>>();
                if !extras.is_empty() {
                    return Err(SourceDecodeError::UnexpectedFields {
                        fields: extras.into_boxed_slice(),
                    });
                }
            }
            Ok(RuntimeValue::Record(decoded))
        }
        DecodeProgramStep::Sum { variants, .. } => {
            let (name, payload) = match value {
                ExternalSourceValue::Variant { name, payload } => {
                    (name.as_ref(), payload.as_deref())
                }
                ExternalSourceValue::Text(name) => (name.as_ref(), None),
                other => return Err(type_mismatch("explicit sum variant", other)),
            };
            let Some(variant) = variants
                .iter()
                .find(|variant| variant.name.as_str() == name)
            else {
                return Err(SourceDecodeError::UnknownVariant {
                    found: name.into(),
                    expected: variants
                        .iter()
                        .map(|variant| variant.name.as_str().into())
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                });
            };
            let fields = match (variant.payload, payload) {
                (None, None) => Vec::new(),
                (None, Some(_)) => {
                    return Err(SourceDecodeError::UnexpectedVariantPayload {
                        variant: variant.name.as_str().into(),
                    });
                }
                (Some(_), None) => {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: variant.name.as_str().into(),
                    });
                }
                (Some(payload_step), Some(payload)) => {
                    let decoded =
                        decode_step(program, program.step(payload_step), payload, depth + 1)?;
                    match program.step(payload_step) {
                        DecodeProgramStep::Tuple { .. } => match decoded {
                            RuntimeValue::Tuple(fields) => fields,
                            tuple => vec![tuple],
                        },
                        _ => vec![decoded],
                    }
                }
            };
            let (item, type_name) = match &variant.constructor {
                Some(constructor) => (constructor.item, constructor.type_name.clone()),
                None => (aivi_hir::ItemId::from_raw(0), "<decoded-sum>".into()),
            };
            Ok(RuntimeValue::Sum(RuntimeSumValue {
                item,
                type_name,
                variant_name: variant.name.as_str().into(),
                fields,
            }))
        }
        DecodeProgramStep::Domain { surface, .. } => Err(SourceDecodeError::UnsupportedProgram(
            SourceDecodeProgramSupportError::UnsupportedDomain {
                member_name: surface.member_name.clone(),
            },
        )),
        DecodeProgramStep::List { element } => {
            let ExternalSourceValue::List(values) = value else {
                return Err(type_mismatch("list", value));
            };
            let decoded = values
                .iter()
                .map(|value| decode_step(program, program.step(*element), value, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(RuntimeValue::List(decoded))
        }
        DecodeProgramStep::Option { element } => match value {
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "None" => {
                if payload.is_some() {
                    return Err(SourceDecodeError::UnexpectedVariantPayload {
                        variant: name.clone(),
                    });
                }
                Ok(RuntimeValue::OptionNone)
            }
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "Some" => {
                let Some(payload) = payload.as_deref() else {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: name.clone(),
                    });
                };
                Ok(RuntimeValue::OptionSome(Box::new(decode_step(
                    program,
                    program.step(*element),
                    payload,
                    depth + 1,
                )?)))
            }
            other => Err(type_mismatch("option variant", other)),
        },
        DecodeProgramStep::Result {
            error,
            value: value_step,
        } => match value {
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "Ok" => {
                let Some(payload) = payload.as_deref() else {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: name.clone(),
                    });
                };
                Ok(RuntimeValue::ResultOk(Box::new(decode_step(
                    program,
                    program.step(*value_step),
                    payload,
                    depth + 1,
                )?)))
            }
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "Err" => {
                let Some(payload) = payload.as_deref() else {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: name.clone(),
                    });
                };
                Ok(RuntimeValue::ResultErr(Box::new(decode_step(
                    program,
                    program.step(*error),
                    payload,
                    depth + 1,
                )?)))
            }
            other => Err(type_mismatch("result variant", other)),
        },
        DecodeProgramStep::Validation {
            error,
            value: value_step,
        } => match value {
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "Valid" => {
                let Some(payload) = payload.as_deref() else {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: name.clone(),
                    });
                };
                Ok(RuntimeValue::ValidationValid(Box::new(decode_step(
                    program,
                    program.step(*value_step),
                    payload,
                    depth + 1,
                )?)))
            }
            ExternalSourceValue::Variant { name, payload } if name.as_ref() == "Invalid" => {
                let Some(payload) = payload.as_deref() else {
                    return Err(SourceDecodeError::MissingVariantPayload {
                        variant: name.clone(),
                    });
                };
                Ok(RuntimeValue::ValidationInvalid(Box::new(decode_step(
                    program,
                    program.step(*error),
                    payload,
                    depth + 1,
                )?)))
            }
            other => Err(type_mismatch("validation variant", other)),
        },
    }
}

fn type_mismatch(expected: &'static str, value: &ExternalSourceValue) -> SourceDecodeError {
    SourceDecodeError::TypeMismatch {
        expected,
        found: value_kind(value),
    }
}

fn value_kind(value: &ExternalSourceValue) -> &'static str {
    match value {
        ExternalSourceValue::Unit => "unit",
        ExternalSourceValue::Bool(_) => "bool",
        ExternalSourceValue::Int(_) => "integer",
        ExternalSourceValue::Float(_) => "float",
        ExternalSourceValue::Text(_) => "text",
        ExternalSourceValue::List(_) => "list",
        ExternalSourceValue::Record(_) => "record",
        ExternalSourceValue::Variant { .. } => "variant",
    }
}

#[cfg(test)]
mod tests {
    use aivi_backend::{
        RuntimeBigInt, RuntimeDecimal, RuntimeFloat, RuntimeSumValue, RuntimeValue,
    };
    use aivi_base::SourceDatabase;
    use aivi_hir::{
        Item, SourceDecodeProgramOutcome, generate_source_decode_programs, lower_module,
    };
    use aivi_syntax::parse_module;

    use super::{
        ExternalSourceValue, SourceDecodeError, decode_external, encode_runtime_json,
        parse_json_text,
    };

    fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn item_name(module: &aivi_hir::Module, item_id: aivi_hir::ItemId) -> &str {
        match &module.items()[item_id] {
            Item::Type(item) => item.name.text(),
            Item::Value(item) => item.name.text(),
            Item::Function(item) => item.name.text(),
            Item::Signal(item) => item.name.text(),
            Item::Class(item) => item.name.text(),
            Item::Domain(item) => item.name.text(),
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => "<anonymous>",
        }
    }

    fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| {
                (item_name(module, item_id) == name).then_some((item_id, item))
            })
            .map(|(item_id, _)| item_id)
            .unwrap_or_else(|| panic!("expected item named `{name}`"))
    }

    fn lowered_decode_program(
        path: &str,
        text: &str,
        signal_name: &str,
    ) -> (aivi_hir::LoweringResult, aivi_hir::SourceDecodeProgram) {
        let lowered = lower_text(path, text);
        assert!(
            !lowered.has_errors(),
            "decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = generate_source_decode_programs(lowered.module());
        let node = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == signal_name)
            .unwrap_or_else(|| panic!("expected decode program for {signal_name}"));
        let program = match &node.outcome {
            SourceDecodeProgramOutcome::Planned(program) => program.clone(),
            other => panic!("expected planned decode program, found {other:?}"),
        };
        (lowered, program)
    }

    fn decode_program(path: &str, text: &str, signal_name: &str) -> aivi_hir::SourceDecodeProgram {
        lowered_decode_program(path, text, signal_name).1
    }

    #[test]
    fn decodes_float_source_payloads_from_json_numbers() {
        let program = decode_program(
            "source-decode-float.aivi",
            r#"
@source custom.feed
signal temperature : Signal Float
"#,
            "temperature",
        );

        let float = decode_external(
            &program,
            &parse_json_text("1.5").expect("float JSON should parse"),
        )
        .expect("float JSON should decode");
        assert_eq!(
            float,
            RuntimeValue::Float(RuntimeFloat::new(1.5).expect("finite float should construct"))
        );

        let promoted_int = decode_external(
            &program,
            &parse_json_text("1").expect("integer JSON should parse"),
        )
        .expect("integer JSON should promote into Float when the signal expects Float");
        assert_eq!(
            promoted_int,
            RuntimeValue::Float(RuntimeFloat::new(1.0).expect("finite float should construct"))
        );
    }

    #[test]
    fn encodes_float_runtime_values_into_json_numbers() {
        let encoded = encode_runtime_json(&RuntimeValue::Float(
            RuntimeFloat::new(1.5).expect("finite float should construct"),
        ))
        .expect("float runtime values should encode into JSON");
        assert_eq!(encoded, "1.5");
    }

    #[test]
    fn decodes_decimal_and_bigint_source_payloads_from_json_strings() {
        let decimal_program = decode_program(
            "source-decode-decimal.aivi",
            r#"
@source custom.feed
signal price : Signal Decimal
"#,
            "price",
        );
        let decimal = decode_external(
            &decimal_program,
            &parse_json_text(r#""19.25d""#).expect("decimal JSON string should parse"),
        )
        .expect("decimal JSON string should decode");
        assert_eq!(
            decimal,
            RuntimeValue::Decimal(
                RuntimeDecimal::parse_literal("19.25d")
                    .expect("decimal literal should parse into the runtime type"),
            )
        );

        let bigint_program = decode_program(
            "source-decode-bigint.aivi",
            r#"
@source custom.feed
signal count : Signal BigInt
"#,
            "count",
        );
        let bigint = decode_external(
            &bigint_program,
            &parse_json_text(r#""123456789012345678901234567890n""#)
                .expect("bigint JSON string should parse"),
        )
        .expect("bigint JSON string should decode");
        assert_eq!(
            bigint,
            RuntimeValue::BigInt(
                RuntimeBigInt::parse_literal("123456789012345678901234567890n")
                    .expect("bigint literal should parse into the runtime type"),
            )
        );
    }

    #[test]
    fn decodes_bytes_source_payloads_from_json_octet_arrays() {
        let program = decode_program(
            "source-decode-bytes.aivi",
            r#"
@source custom.feed
signal bytes : Signal Bytes
"#,
            "bytes",
        );

        let decoded = decode_external(
            &program,
            &parse_json_text("[104, 105, 33]").expect("byte-array JSON should parse"),
        )
        .expect("byte-array JSON should decode");
        assert_eq!(decoded, RuntimeValue::Bytes(Box::new([104, 105, 33])));
    }

    #[test]
    fn decodes_same_module_text_wrapper_sums_with_real_constructor_identity() {
        let (lowered, program) = lowered_decode_program(
            "source-decode-window-key-wrapper.aivi",
            r#"
type Key =
  | Key Text

@source window.keyDown with {
    repeat: False
    focusOnly: True
}
signal keyDown : Signal Key
"#,
            "keyDown",
        );

        let decoded = decode_external(
            &program,
            &ExternalSourceValue::variant_with_payload(
                "Key",
                ExternalSourceValue::Text("ArrowDown".into()),
            ),
        )
        .expect("wrapped key payload should decode");

        let constructor = lowered
            .module()
            .sum_constructor_handle(item_id(lowered.module(), "Key"), "Key")
            .expect("same-module Key constructor should resolve");
        assert_eq!(
            decoded,
            RuntimeValue::Sum(RuntimeSumValue {
                item: constructor.item,
                type_name: constructor.type_name.clone(),
                variant_name: constructor.variant_name.clone(),
                fields: vec![RuntimeValue::Text("ArrowDown".into())],
            })
        );
    }

    #[test]
    fn encodes_decimal_bigint_and_bytes_runtime_values_into_json() {
        let decimal = encode_runtime_json(&RuntimeValue::Decimal(
            RuntimeDecimal::parse_literal("19.25d").expect("decimal literal should parse"),
        ))
        .expect("decimal runtime values should encode into JSON");
        assert_eq!(decimal, r#""19.25d""#);

        let bigint = encode_runtime_json(&RuntimeValue::BigInt(
            RuntimeBigInt::parse_literal("123n").expect("bigint literal should parse"),
        ))
        .expect("bigint runtime values should encode into JSON");
        assert_eq!(bigint, r#""123n""#);

        let bytes = encode_runtime_json(&RuntimeValue::Bytes(Box::new([104, 105, 33])))
            .expect("byte runtime values should encode into JSON");
        assert_eq!(bytes, "[104,105,33]");
    }

    #[test]
    fn rejects_invalid_decimal_and_bytes_source_payload_shapes_explicitly() {
        let decimal_program = decode_program(
            "source-decode-invalid-decimal.aivi",
            r#"
@source custom.feed
signal price : Signal Decimal
"#,
            "price",
        );
        let decimal_error = decode_external(
            &decimal_program,
            &parse_json_text(r#""19.25""#).expect("invalid decimal JSON string should still parse"),
        )
        .expect_err("missing decimal suffix should be rejected explicitly");
        assert_eq!(
            decimal_error,
            SourceDecodeError::InvalidScalarLiteral {
                scalar: "Decimal",
                value: "19.25".into(),
            }
        );

        let bytes_program = decode_program(
            "source-decode-invalid-bytes.aivi",
            r#"
@source custom.feed
signal bytes : Signal Bytes
"#,
            "bytes",
        );
        let bytes_error = decode_external(
            &bytes_program,
            &parse_json_text("[256]").expect("out-of-range byte array should still parse"),
        )
        .expect_err("byte values above 255 should be rejected explicitly");
        assert_eq!(
            bytes_error,
            SourceDecodeError::InvalidByteValue {
                index: 0,
                value: 256,
            }
        );
    }
}
