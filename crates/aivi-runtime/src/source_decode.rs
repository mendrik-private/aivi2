use std::collections::{BTreeMap, BTreeSet};

use aivi_backend::{RuntimeFloat, RuntimeRecordField, RuntimeSumValue, RuntimeValue};
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
                | aivi_typing::PrimitiveType::Text => {}
                aivi_typing::PrimitiveType::Decimal => {
                    return Err(SourceDecodeProgramSupportError::UnsupportedScalar {
                        scalar: "Decimal",
                    });
                }
                aivi_typing::PrimitiveType::BigInt => {
                    return Err(SourceDecodeProgramSupportError::UnsupportedScalar {
                        scalar: "BigInt",
                    });
                }
                aivi_typing::PrimitiveType::Bytes => {
                    return Err(SourceDecodeProgramSupportError::UnsupportedScalar {
                        scalar: "Bytes",
                    });
                }
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

pub fn decode_external(
    program: &SourceDecodeProgram,
    value: &ExternalSourceValue,
) -> Result<RuntimeValue, SourceDecodeError> {
    validate_supported_program(program).map_err(SourceDecodeError::UnsupportedProgram)?;
    decode_step(program, program.root_step(), value)
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
        RuntimeValue::Decimal(_) => {
            Err("runtime JSON encoding does not support Decimal values yet".into())
        }
        RuntimeValue::BigInt(_) => {
            Err("runtime JSON encoding does not support BigInt values yet".into())
        }
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
            for entry in entries {
                let Some(key) = entry.key.as_text() else {
                    return Err("runtime JSON encoding requires Text map keys".into());
                };
                object.insert(key.to_owned(), runtime_to_json(&entry.value)?);
            }
            Ok(JsonValue::Object(object))
        }
        RuntimeValue::Callable(_) => {
            Err("runtime JSON encoding does not support callable values".into())
        }
        RuntimeValue::Bytes(_) => {
            Err("runtime JSON encoding does not support Bytes values yet".into())
        }
        RuntimeValue::Task(_) => Err("runtime JSON encoding does not support Task values".into()),
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

fn decode_step(
    program: &SourceDecodeProgram,
    step: &DecodeProgramStep,
    value: &ExternalSourceValue,
) -> Result<RuntimeValue, SourceDecodeError> {
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
            aivi_typing::PrimitiveType::Decimal => Err(SourceDecodeError::UnsupportedProgram(
                SourceDecodeProgramSupportError::UnsupportedScalar { scalar: "Decimal" },
            )),
            aivi_typing::PrimitiveType::BigInt => Err(SourceDecodeError::UnsupportedProgram(
                SourceDecodeProgramSupportError::UnsupportedScalar { scalar: "BigInt" },
            )),
            aivi_typing::PrimitiveType::Bytes => Err(SourceDecodeError::UnsupportedProgram(
                SourceDecodeProgramSupportError::UnsupportedScalar { scalar: "Bytes" },
            )),
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
                .map(|(element, value)| decode_step(program, program.step(*element), value))
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
                    value: decode_step(program, program.step(field.step), value)?,
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
                    let decoded = decode_step(program, program.step(payload_step), payload)?;
                    match program.step(payload_step) {
                        DecodeProgramStep::Tuple { .. } => match decoded {
                            RuntimeValue::Tuple(fields) => fields,
                            tuple => vec![tuple],
                        },
                        _ => vec![decoded],
                    }
                }
            };
            Ok(RuntimeValue::Sum(RuntimeSumValue {
                item: aivi_hir::ItemId::from_raw(0),
                type_name: "<decoded-sum>".into(),
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
                .map(|value| decode_step(program, program.step(*element), value))
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
    use aivi_base::SourceDatabase;
    use aivi_backend::{RuntimeFloat, RuntimeValue};
    use aivi_hir::{Item, SourceDecodeProgramOutcome, generate_source_decode_programs, lower_module};
    use aivi_syntax::parse_module;

    use super::{decode_external, encode_runtime_json, parse_json_text};

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

    fn decode_program(path: &str, text: &str, signal_name: &str) -> aivi_hir::SourceDecodeProgram {
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
        match &node.outcome {
            SourceDecodeProgramOutcome::Planned(program) => program.clone(),
            other => panic!("expected planned decode program, found {other:?}"),
        }
    }

    #[test]
    fn decodes_float_source_payloads_from_json_numbers() {
        let program = decode_program(
            "source-decode-float.aivi",
            r#"
@source custom.feed
sig temperature : Signal Float
"#,
            "temperature",
        );

        let float = decode_external(&program, &parse_json_text("1.5").expect("float JSON should parse"))
            .expect("float JSON should decode");
        assert_eq!(
            float,
            RuntimeValue::Float(RuntimeFloat::new(1.5).expect("finite float should construct"))
        );

        let promoted_int =
            decode_external(&program, &parse_json_text("1").expect("integer JSON should parse"))
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
}
