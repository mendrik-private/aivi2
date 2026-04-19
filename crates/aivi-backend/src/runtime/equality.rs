pub(crate) fn normalize_signal_kernel_result(
    program: &Program,
    kernel: KernelId,
    raw_result: RuntimeValue,
    expected: LayoutId,
) -> Result<RuntimeValue, EvaluationError> {
    let result = match (&program.layouts()[expected].kind, raw_result) {
        (LayoutKind::Signal { element }, value) if value_matches_layout(program, &value, *element) => {
            value
        }
        (_, RuntimeValue::Signal(value)) if value_matches_layout(program, value.as_ref(), expected) => {
            *value
        }
        (_, value) => value,
    };
    if !value_matches_layout(program, &result, expected) {
        return Err(EvaluationError::KernelResultLayoutMismatch {
            kernel,
            expected,
            found: result,
        });
    }
    Ok(result)
}

pub(crate) fn value_matches_layout_with_signal_current(
    program: &Program,
    value: &RuntimeValue,
    layout: LayoutId,
) -> bool {
    value_matches_layout(program, value, layout)
        || matches!(value, RuntimeValue::Signal(inner) if value_matches_layout(program, inner, layout))
}

pub(crate) fn value_matches_layout(program: &Program, value: &RuntimeValue, layout: LayoutId) -> bool {
    let Some(layout) = program.layouts().get(layout) else {
        return false;
    };
    match (&layout.kind, value) {
        (LayoutKind::Primitive(PrimitiveType::Unit), RuntimeValue::Unit) => true,
        (LayoutKind::Primitive(PrimitiveType::Bool), RuntimeValue::Bool(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Int), RuntimeValue::Int(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Float), RuntimeValue::Float(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Decimal), RuntimeValue::Decimal(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::BigInt), RuntimeValue::BigInt(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Text), RuntimeValue::Text(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Bytes), RuntimeValue::Bytes(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Task), RuntimeValue::Task(_))
        | (LayoutKind::Primitive(PrimitiveType::Task), RuntimeValue::DbTask(_))
        | (LayoutKind::Task { .. }, RuntimeValue::Task(_))
        | (LayoutKind::Task { .. }, RuntimeValue::DbTask(_)) => true,
        (LayoutKind::Tuple(expected), RuntimeValue::Tuple(elements)) => {
            expected.len() == elements.len()
                && expected
                    .iter()
                    .zip(elements.iter())
                    .all(|(layout, value)| value_matches_layout(program, value, *layout))
        }
        // Shallow tag checks: the typechecker guarantees element types are correct, so
        // walking every element on every kernel call would be O(N) per check and catastrophic
        // for large collections (e.g. Matrix = List(List(...))).
        (LayoutKind::List { .. }, RuntimeValue::List(_))
        | (LayoutKind::Set { .. }, RuntimeValue::Set(_)) => true,
        (LayoutKind::Map { .. }, RuntimeValue::Map(_)) => true,
        (LayoutKind::Record(expected), RuntimeValue::Record(fields)) => {
            expected.len() == fields.len()
                && expected.iter().zip(fields.iter()).all(|(layout, field)| {
                    layout.name.as_ref() == field.label.as_ref()
                        && value_matches_layout(program, &field.value, layout.layout)
                })
        }
        (LayoutKind::Sum(variants), RuntimeValue::Sum(value)) => variants
            .iter()
            .find(|variant| variant.name.as_ref() == value.variant_name.as_ref())
            .is_some_and(|variant| {
                sum_fields_match_layout(program, &value.fields, variant.payload)
            }),
        (LayoutKind::Option { element }, RuntimeValue::OptionNone) => {
            let _ = element;
            true
        }
        (LayoutKind::Option { element }, RuntimeValue::OptionSome(value)) => {
            value_matches_layout(program, value, *element)
        }
        (LayoutKind::Result { value, .. }, RuntimeValue::ResultOk(result)) => {
            value_matches_layout(program, result, *value)
        }
        (LayoutKind::Result { error, .. }, RuntimeValue::ResultErr(result)) => {
            value_matches_layout(program, result, *error)
        }
        (LayoutKind::Validation { value, .. }, RuntimeValue::ValidationValid(result)) => {
            value_matches_layout(program, result, *value)
        }
        (LayoutKind::Validation { error, .. }, RuntimeValue::ValidationInvalid(result)) => {
            value_matches_layout(program, result, *error)
        }
        (LayoutKind::Signal { element }, RuntimeValue::Signal(value)) => {
            value_matches_layout(program, value, *element)
        }
        (LayoutKind::Signal { element }, value) => value_matches_layout(program, value, *element),
        (LayoutKind::Arrow { .. }, RuntimeValue::Callable(_)) => true,
        (LayoutKind::AnonymousDomain { .. }, RuntimeValue::SuffixedInteger { .. }) => true,
        (LayoutKind::Domain { .. }, RuntimeValue::Signal(_)) => false,
        // Named-domain layouts erase their carrier shape in backend IR. Runtime evaluation relies
        // on earlier typed lowering to keep those carrier values sound and only preserves the
        // outer signal/non-signal distinction here.
        (LayoutKind::Domain { .. }, _) => true,
        (LayoutKind::Opaque { name, variants, .. }, RuntimeValue::Sum(value)) => {
            name.as_ref() == value.type_name.as_ref()
                && (variants.is_empty()
                    || variants
                        .iter()
                        .find(|variant| variant.name.as_ref() == value.variant_name.as_ref())
                        .is_some_and(|variant| {
                            sum_fields_match_layout(program, &value.fields, variant.payload)
                        }))
        }
        _ => false,
    }
}

fn sum_fields_match_layout(
    program: &Program,
    fields: &[RuntimeValue],
    payload: Option<LayoutId>,
) -> bool {
    match (payload, fields) {
        (None, []) => true,
        (Some(layout), [field]) => value_matches_layout(program, field, layout),
        (Some(layout), fields) if fields.len() > 1 => {
            let Some(layout) = program.layouts().get(layout) else {
                return false;
            };
            let LayoutKind::Tuple(expected) = &layout.kind else {
                return false;
            };
            expected.len() == fields.len()
                && expected
                    .iter()
                    .zip(fields.iter())
                    .all(|(layout, field)| value_matches_layout(program, field, *layout))
        }
        _ => false,
    }
}

fn structural_eq(
    kernel: KernelId,
    expr: KernelExprId,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<bool, EvaluationError> {
    if let RuntimeValue::Signal(inner) = left {
        return structural_eq(kernel, expr, inner, right);
    }
    if let RuntimeValue::Signal(inner) = right {
        return structural_eq(kernel, expr, left, inner);
    }
    let equal = match (left, right) {
        (RuntimeValue::Unit, RuntimeValue::Unit) => true,
        (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => left == right,
        (RuntimeValue::Int(left), RuntimeValue::Int(right)) => left == right,
        (RuntimeValue::Float(left), RuntimeValue::Float(right)) => left == right,
        (RuntimeValue::Decimal(left), RuntimeValue::Decimal(right)) => left == right,
        (RuntimeValue::BigInt(left), RuntimeValue::BigInt(right)) => left == right,
        (RuntimeValue::Text(left), RuntimeValue::Text(right)) => left == right,
        (RuntimeValue::Bytes(left), RuntimeValue::Bytes(right)) => left == right,
        (RuntimeValue::Int(left), RuntimeValue::SuffixedInteger { raw, .. })
        | (RuntimeValue::SuffixedInteger { raw, .. }, RuntimeValue::Int(left)) => {
            raw.parse::<i64>().ok() == Some(*left)
        }
        (
            RuntimeValue::SuffixedInteger {
                raw: left_raw,
                suffix: left_suffix,
            },
            RuntimeValue::SuffixedInteger {
                raw: right_raw,
                suffix: right_suffix,
            },
        ) => left_raw == right_raw && left_suffix == right_suffix,
        (RuntimeValue::Tuple(left), RuntimeValue::Tuple(right))
        | (RuntimeValue::List(left), RuntimeValue::List(right)) => {
            if left.len() != right.len() {
                false
            } else {
                for (left, right) in left.iter().zip(right.iter()) {
                    if !structural_eq(kernel, expr, left, right)? {
                        return Ok(false);
                    }
                }
                true
            }
        }
        (RuntimeValue::Set(left), RuntimeValue::Set(right)) => {
            unordered_runtime_values_eq(kernel, expr, left, right)?
        }
        (RuntimeValue::Map(left), RuntimeValue::Map(right)) => {
            unordered_runtime_map_eq(kernel, expr, left, right)?
        }
        (RuntimeValue::Record(left), RuntimeValue::Record(right)) => {
            if left.len() != right.len() {
                false
            } else {
                for (left, right) in left.iter().zip(right.iter()) {
                    if left.label != right.label
                        || !structural_eq(kernel, expr, &left.value, &right.value)?
                    {
                        return Ok(false);
                    }
                }
                true
            }
        }
        (RuntimeValue::Sum(left), RuntimeValue::Sum(right)) => {
            if left.item != right.item
                || left.variant_name != right.variant_name
                || left.fields.len() != right.fields.len()
            {
                false
            } else {
                for (left, right) in left.fields.iter().zip(right.fields.iter()) {
                    if !structural_eq(kernel, expr, left, right)? {
                        return Ok(false);
                    }
                }
                true
            }
        }
        (RuntimeValue::OptionNone, RuntimeValue::OptionNone) => true,
        (RuntimeValue::OptionSome(left), RuntimeValue::OptionSome(right))
        | (RuntimeValue::ResultOk(left), RuntimeValue::ResultOk(right))
        | (RuntimeValue::ResultErr(left), RuntimeValue::ResultErr(right))
        | (RuntimeValue::ValidationValid(left), RuntimeValue::ValidationValid(right))
        | (RuntimeValue::ValidationInvalid(left), RuntimeValue::ValidationInvalid(right))
        | (RuntimeValue::Signal(left), RuntimeValue::Signal(right)) => {
            structural_eq(kernel, expr, left, right)?
        }
        (RuntimeValue::Callable(_), _)
        | (_, RuntimeValue::Callable(_))
        | (RuntimeValue::Task(_), _)
        | (_, RuntimeValue::Task(_))
        | (RuntimeValue::DbTask(_), _)
        | (_, RuntimeValue::DbTask(_)) => {
            return Err(EvaluationError::UnsupportedStructuralEquality {
                kernel,
                expr,
                left: left.clone(),
                right: right.clone(),
            });
        }
        _ => false,
    };
    Ok(equal)
}

fn unordered_runtime_values_eq(
    kernel: KernelId,
    expr: KernelExprId,
    left: &[RuntimeValue],
    right: &[RuntimeValue],
) -> Result<bool, EvaluationError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    let mut matched = vec![false; right.len()];
    'left_values: for left_value in left {
        for (index, right_value) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if !runtime_values_may_match(left_value, right_value) {
                continue;
            }
            if structural_eq(kernel, expr, left_value, right_value)? {
                matched[index] = true;
                continue 'left_values;
            }
        }
        return Ok(false);
    }
    Ok(true)
}

fn unordered_runtime_map_eq(
    kernel: KernelId,
    expr: KernelExprId,
    left: &RuntimeMap,
    right: &RuntimeMap,
) -> Result<bool, EvaluationError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    // Use O(1) key lookup on `right` to drive the comparison in O(n) rather
    // than the previous O(n²) linear scan.  Both sides must agree on every
    // key, and the associated values must be structurally equal.
    for (left_key, left_value) in left {
        let Some(right_value) = right.get(left_key) else {
            return Ok(false);
        };
        if !structural_eq(kernel, expr, left_value, right_value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn runtime_values_may_match(left: &RuntimeValue, right: &RuntimeValue) -> bool {
    match (left, right) {
        (RuntimeValue::Signal(left), right) => runtime_values_may_match(left, right),
        (left, RuntimeValue::Signal(right)) => runtime_values_may_match(left, right),
        (RuntimeValue::Unit, RuntimeValue::Unit)
        | (RuntimeValue::Bool(_), RuntimeValue::Bool(_))
        | (RuntimeValue::Int(_), RuntimeValue::Int(_))
        | (RuntimeValue::Float(_), RuntimeValue::Float(_))
        | (RuntimeValue::Decimal(_), RuntimeValue::Decimal(_))
        | (RuntimeValue::BigInt(_), RuntimeValue::BigInt(_))
        | (RuntimeValue::Text(_), RuntimeValue::Text(_))
        | (RuntimeValue::Bytes(_), RuntimeValue::Bytes(_))
        | (RuntimeValue::Tuple(_), RuntimeValue::Tuple(_))
        | (RuntimeValue::List(_), RuntimeValue::List(_))
        | (RuntimeValue::Set(_), RuntimeValue::Set(_))
        | (RuntimeValue::Map(_), RuntimeValue::Map(_))
        | (RuntimeValue::Record(_), RuntimeValue::Record(_))
        | (RuntimeValue::Sum(_), RuntimeValue::Sum(_))
        | (RuntimeValue::OptionNone, RuntimeValue::OptionNone)
        | (RuntimeValue::OptionSome(_), RuntimeValue::OptionSome(_))
        | (RuntimeValue::ResultOk(_), RuntimeValue::ResultOk(_))
        | (RuntimeValue::ResultErr(_), RuntimeValue::ResultErr(_))
        | (RuntimeValue::ValidationValid(_), RuntimeValue::ValidationValid(_))
        | (RuntimeValue::ValidationInvalid(_), RuntimeValue::ValidationInvalid(_))
        | (RuntimeValue::Task(_), RuntimeValue::Task(_))
        | (RuntimeValue::DbTask(_), RuntimeValue::DbTask(_))
        | (RuntimeValue::Callable(_), RuntimeValue::Callable(_))
        | (RuntimeValue::SuffixedInteger { .. }, RuntimeValue::SuffixedInteger { .. }) => true,
        (RuntimeValue::Int(_), RuntimeValue::SuffixedInteger { .. })
        | (RuntimeValue::SuffixedInteger { .. }, RuntimeValue::Int(_)) => true,
        _ => false,
    }
}

fn project_field(
    kernel: KernelId,
    expr: KernelExprId,
    value: RuntimeValue,
    label: &str,
) -> Result<RuntimeValue, EvaluationError> {
    let value = strip_signal(value);
    let RuntimeValue::Record(fields) = value else {
        return Err(EvaluationError::InvalidProjectionBase {
            kernel,
            expr,
            found: value,
        });
    };
    fields
        .into_iter()
        .find(|field| field.label.as_ref() == label)
        .map(|field| field.value)
        .ok_or_else(|| EvaluationError::UnknownProjectionField {
            kernel,
            expr,
            label: label.into(),
        })
}

fn pop_value(values: &mut Vec<RuntimeValue>) -> RuntimeValue {
    values
        .pop()
        .expect("backend runtime evaluation should keep task/value stacks aligned")
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("backend runtime evaluation should not underflow its value stack");
    values.split_off(split)
}

fn truthy_falsy_payload(
    value: &RuntimeValue,
    constructor: BuiltinTerm,
) -> Option<Option<RuntimeValue>> {
    match (constructor, value) {
        (BuiltinTerm::True, RuntimeValue::Bool(true))
        | (BuiltinTerm::False, RuntimeValue::Bool(false))
        | (BuiltinTerm::None, RuntimeValue::OptionNone) => Some(None),
        (BuiltinTerm::Some, RuntimeValue::OptionSome(payload))
        | (BuiltinTerm::Ok, RuntimeValue::ResultOk(payload))
        | (BuiltinTerm::Err, RuntimeValue::ResultErr(payload))
        | (BuiltinTerm::Valid, RuntimeValue::ValidationValid(payload))
        | (BuiltinTerm::Invalid, RuntimeValue::ValidationInvalid(payload)) => {
            Some(Some((**payload).clone()))
        }
        _ => None,
    }
}

pub fn coerce_runtime_value(
    program: &Program,
    value: RuntimeValue,
    layout: LayoutId,
) -> Result<RuntimeValue, RuntimeValue> {
    let Some(layout_def) = program.layouts().get(layout) else {
        return Err(value);
    };
    if let LayoutKind::Signal { element } = &layout_def.kind {
        let inner = match value {
            RuntimeValue::Signal(inner) => *inner,
            other => other,
        };
        return coerce_runtime_value(program, inner, *element)
            .map(|inner| RuntimeValue::Signal(Box::new(inner)));
    }
    if value_matches_layout(program, &value, layout) {
        return Ok(value);
    }
    if let RuntimeValue::Signal(inner) = &value {
        let payload = inner.as_ref().clone();
        if value_matches_layout(program, &payload, layout) {
            return Ok(payload);
        }
    }
    match &layout_def.kind {
        LayoutKind::Option { element } => {
            if value_matches_layout(program, &value, *element) {
                Ok(RuntimeValue::OptionSome(Box::new(value)))
            } else {
                Err(value)
            }
        }
        LayoutKind::Result { value: ok, error } => {
            let matches_ok = value_matches_layout(program, &value, *ok);
            let matches_err = value_matches_layout(program, &value, *error);
            match (matches_ok, matches_err) {
                (true, false) => Ok(RuntimeValue::ResultOk(Box::new(value))),
                (false, true) => Ok(RuntimeValue::ResultErr(Box::new(value))),
                _ => Err(value),
            }
        }
        LayoutKind::Validation {
            value: valid,
            error: invalid,
        } => {
            let matches_valid = value_matches_layout(program, &value, *valid);
            let matches_invalid = value_matches_layout(program, &value, *invalid);
            match (matches_valid, matches_invalid) {
                (true, false) => Ok(RuntimeValue::ValidationValid(Box::new(value))),
                (false, true) => Ok(RuntimeValue::ValidationInvalid(Box::new(value))),
                _ => Err(value),
            }
        }
        _ => Err(value),
    }
}

fn coerce_inline_pipe_value(
    program: &Program,
    value: RuntimeValue,
    layout: LayoutId,
) -> Option<RuntimeValue> {
    coerce_runtime_value(program, value, layout).ok()
}

pub(crate) fn strip_signal(value: RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(value) => *value,
        other => other,
    }
}

fn append_validation_errors(
    left: RuntimeValue,
    right: RuntimeValue,
) -> Result<RuntimeValue, &'static str> {
    let RuntimeValue::Sum(left) = left else {
        return Err(
            "Validation apply only accumulates Invalid payloads shaped as `NonEmpty`/`NonEmptyList`",
        );
    };
    let RuntimeValue::Sum(right) = right else {
        return Err(
            "Validation apply only accumulates Invalid payloads shaped as `NonEmpty`/`NonEmptyList`",
        );
    };
    if !matches_non_empty_runtime(&left) || !matches_non_empty_runtime(&right) {
        return Err(
            "Validation apply only accumulates Invalid payloads shaped as `NonEmpty`/`NonEmptyList`",
        );
    }

    let RuntimeSumValue {
        item,
        type_name,
        variant_name,
        fields: left_fields,
    } = left;
    let mut left_fields = left_fields;
    let head = left_fields.remove(0);
    let left_tail = match left_fields.remove(0) {
        RuntimeValue::List(values) => values,
        _ => {
            return Err(
                "Validation apply only accumulates Invalid payloads shaped as `NonEmpty`/`NonEmptyList`",
            );
        }
    };

    let RuntimeSumValue {
        fields: right_fields,
        ..
    } = right;
    let mut right_fields = right_fields;
    let right_head = right_fields.remove(0);
    let right_tail = match right_fields.remove(0) {
        RuntimeValue::List(values) => values,
        _ => {
            return Err(
                "Validation apply only accumulates Invalid payloads shaped as `NonEmpty`/`NonEmptyList`",
            );
        }
    };

    let mut tail = left_tail;
    tail.push(right_head);
    tail.extend(right_tail);

    Ok(RuntimeValue::Sum(RuntimeSumValue {
        item,
        type_name,
        variant_name,
        fields: vec![head, RuntimeValue::List(tail)],
    }))
}

fn matches_non_empty_runtime(value: &RuntimeSumValue) -> bool {
    matches!(value.type_name.as_ref(), "NonEmpty" | "NonEmptyList")
        && matches!(value.variant_name.as_ref(), "NonEmpty" | "NonEmptyList")
        && value.fields.len() == 2
        && matches!(value.fields.get(1), Some(RuntimeValue::List(_)))
}
