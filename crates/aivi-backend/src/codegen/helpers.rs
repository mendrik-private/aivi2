fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("static evaluator should never drain more values than it has produced");
    values.drain(split..).collect()
}

fn static_intrinsic_arity(intrinsic: IntrinsicValue) -> Option<usize> {
    match intrinsic {
        IntrinsicValue::BytesLength
        | IntrinsicValue::BytesFromText
        | IntrinsicValue::BytesToText => Some(1),
        IntrinsicValue::BytesGet | IntrinsicValue::BytesAppend | IntrinsicValue::BytesRepeat => {
            Some(2)
        }
        IntrinsicValue::BytesSlice => Some(3),
        _ => None,
    }
}

fn static_evaluate_intrinsic_call(
    intrinsic: IntrinsicValue,
    arguments: Vec<RuntimeValue>,
) -> Option<RuntimeValue> {
    let arguments: Vec<_> = arguments.into_iter().map(static_strip_signal).collect();
    match (intrinsic, arguments.as_slice()) {
        (IntrinsicValue::BytesLength, [RuntimeValue::Bytes(bytes)]) => {
            Some(RuntimeValue::Int(bytes.len() as i64))
        }
        (IntrinsicValue::BytesGet, [RuntimeValue::Int(index), RuntimeValue::Bytes(bytes)]) => Some(
            usize::try_from(*index)
                .ok()
                .and_then(|index| bytes.get(index))
                .map(|&byte| RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(byte as i64))))
                .unwrap_or(RuntimeValue::OptionNone),
        ),
        (
            IntrinsicValue::BytesSlice,
            [
                RuntimeValue::Int(from),
                RuntimeValue::Int(to),
                RuntimeValue::Bytes(bytes),
            ],
        ) => {
            let start = (*from as usize).min(bytes.len());
            let end = (*to as usize).min(bytes.len());
            let end = end.max(start);
            Some(RuntimeValue::Bytes(bytes[start..end].into()))
        }
        (IntrinsicValue::BytesAppend, [RuntimeValue::Bytes(left), RuntimeValue::Bytes(right)]) => {
            let mut combined = left.to_vec();
            combined.extend_from_slice(right.as_ref());
            Some(RuntimeValue::Bytes(combined.into()))
        }
        (IntrinsicValue::BytesFromText, [RuntimeValue::Text(text)]) => {
            Some(RuntimeValue::Bytes(text.as_bytes().into()))
        }
        (IntrinsicValue::BytesToText, [RuntimeValue::Bytes(bytes)]) => Some(
            std::str::from_utf8(bytes.as_ref())
                .ok()
                .map(|text| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(text.into()))))
                .unwrap_or(RuntimeValue::OptionNone),
        ),
        (IntrinsicValue::BytesRepeat, [RuntimeValue::Int(byte), RuntimeValue::Int(count)]) => {
            let byte = (*byte).clamp(0, 255) as u8;
            let count = (*count).max(0) as usize;
            Some(RuntimeValue::Bytes(vec![byte; count].into()))
        }
        _ => None,
    }
}

fn static_strip_signal(value: RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(inner) => *inner,
        other => other,
    }
}

fn static_structural_eq(left: &RuntimeValue, right: &RuntimeValue) -> bool {
    let left = match left {
        RuntimeValue::Signal(inner) => inner.as_ref(),
        other => other,
    };
    let right = match right {
        RuntimeValue::Signal(inner) => inner.as_ref(),
        other => other,
    };
    match (left, right) {
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
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| static_structural_eq(left, right))
        }
        (RuntimeValue::Set(left), RuntimeValue::Set(right)) => {
            static_unordered_values_eq(left, right)
        }
        (RuntimeValue::Map(left), RuntimeValue::Map(right)) => static_unordered_map_eq(left, right),
        (RuntimeValue::Record(left), RuntimeValue::Record(right)) => {
            left.len() == right.len()
                && left.iter().zip(right.iter()).all(|(left, right)| {
                    left.label == right.label && static_structural_eq(&left.value, &right.value)
                })
        }
        (RuntimeValue::Sum(left), RuntimeValue::Sum(right)) => {
            left.item == right.item
                && left.variant_name == right.variant_name
                && left.fields.len() == right.fields.len()
                && left
                    .fields
                    .iter()
                    .zip(right.fields.iter())
                    .all(|(left, right)| static_structural_eq(left, right))
        }
        (RuntimeValue::OptionNone, RuntimeValue::OptionNone) => true,
        (RuntimeValue::OptionSome(left), RuntimeValue::OptionSome(right))
        | (RuntimeValue::ResultOk(left), RuntimeValue::ResultOk(right))
        | (RuntimeValue::ResultErr(left), RuntimeValue::ResultErr(right))
        | (RuntimeValue::ValidationValid(left), RuntimeValue::ValidationValid(right))
        | (RuntimeValue::ValidationInvalid(left), RuntimeValue::ValidationInvalid(right))
        | (RuntimeValue::Signal(left), RuntimeValue::Signal(right)) => {
            static_structural_eq(left, right)
        }
        (RuntimeValue::Callable(_), _)
        | (_, RuntimeValue::Callable(_))
        | (RuntimeValue::Task(_), _)
        | (_, RuntimeValue::Task(_))
        | (RuntimeValue::DbTask(_), _)
        | (_, RuntimeValue::DbTask(_)) => false,
        _ => false,
    }
}

fn static_unordered_values_eq(left: &[RuntimeValue], right: &[RuntimeValue]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut matched = vec![false; right.len()];
    'left_values: for left_value in left {
        for (index, right_value) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if static_structural_eq(left_value, right_value) {
                matched[index] = true;
                continue 'left_values;
            }
        }
        return false;
    }
    true
}

fn static_unordered_map_eq(left: &RuntimeMap, right: &RuntimeMap) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut matched = vec![false; right.len()];
    'left_entries: for (left_key, left_value) in left {
        for (index, (right_key, right_value)) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if static_structural_eq(left_key, right_key)
                && static_structural_eq(left_value, right_value)
            {
                matched[index] = true;
                continue 'left_entries;
            }
        }
        return false;
    }
    true
}

fn domain_member_binary_operator(member_name: &str) -> Option<BinaryOperator> {
    match member_name {
        "+" => Some(BinaryOperator::Add),
        "-" => Some(BinaryOperator::Subtract),
        "*" => Some(BinaryOperator::Multiply),
        "/" => Some(BinaryOperator::Divide),
        "%" => Some(BinaryOperator::Modulo),
        ">" => Some(BinaryOperator::GreaterThan),
        "<" => Some(BinaryOperator::LessThan),
        ">=" => Some(BinaryOperator::GreaterThanOrEqual),
        "<=" => Some(BinaryOperator::LessThanOrEqual),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct AbiShape {
    ty: Type,
    size: u32,
    align: u32,
}

#[derive(Clone, Copy)]
struct ProjectionStep {
    offset: i32,
    layout: LayoutId,
}

fn align_to(offset: u32, align: u32) -> u32 {
    debug_assert!(align.is_power_of_two());
    (offset + (align - 1)) & !(align - 1)
}

fn write_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
    let end = offset + 4;
    bytes[offset..end].copy_from_slice(&value.to_le_bytes());
}

fn opaque_layout_identity_matches(
    left_item: Option<aivi_hir::ItemId>,
    left_name: &str,
    right_item: Option<aivi_hir::ItemId>,
    right_name: &str,
) -> bool {
    match (left_item, right_item) {
        (Some(left_item), Some(right_item)) => left_item == right_item,
        _ => left_name == right_name,
    }
}

fn sum_variant_tag_for_opaque(variant_name: &str) -> i64 {
    crate::layout::opaque_variant_tag(variant_name)
}

fn kernel_symbol_for(program: &Program, kernel_id: KernelId, kernel: &Kernel) -> String {
    format!(
        "aivi_{}_kernel{}_{}",
        sanitize_symbol_component(program.item_name(kernel.origin.item)),
        kernel_id.as_raw(),
        match kernel.origin.kind {
            KernelOriginKind::ItemBody { .. } => "item_body".to_owned(),
            KernelOriginKind::SignalBody { .. } => "signal_body".to_owned(),
            KernelOriginKind::GateTrue { stage_index, .. } => format!("gate_true_s{stage_index}"),
            KernelOriginKind::GateFalse { stage_index, .. } => format!("gate_false_s{stage_index}"),
            KernelOriginKind::SignalFilterPredicate { stage_index, .. } => {
                format!("signal_filter_s{stage_index}")
            }
            KernelOriginKind::PreviousSeed { stage_index, .. } => {
                format!("previous_seed_s{stage_index}")
            }
            KernelOriginKind::DiffFunction { stage_index, .. } => {
                format!("diff_function_s{stage_index}")
            }
            KernelOriginKind::DiffSeed { stage_index, .. } => {
                format!("diff_seed_s{stage_index}")
            }
            KernelOriginKind::DelayDuration { stage_index, .. } => {
                format!("delay_duration_s{stage_index}")
            }
            KernelOriginKind::BurstEvery { stage_index, .. } => {
                format!("burst_every_s{stage_index}")
            }
            KernelOriginKind::BurstCount { stage_index, .. } => {
                format!("burst_count_s{stage_index}")
            }
            KernelOriginKind::FanoutMap { stage_index, .. } => {
                format!("fanout_map_s{stage_index}")
            }
            KernelOriginKind::FanoutFilterPredicate { stage_index, .. } => {
                format!("fanout_filter_s{stage_index}")
            }
            KernelOriginKind::FanoutJoin { stage_index, .. } => {
                format!("fanout_join_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStart { stage_index, .. } => {
                format!("recurrence_start_s{stage_index}")
            }
            KernelOriginKind::RecurrenceStep { stage_index, .. } => {
                format!("recurrence_step_s{stage_index}")
            }
            KernelOriginKind::RecurrenceWakeupWitness { .. } => "recurrence_witness".to_owned(),
            KernelOriginKind::RecurrenceSeed { .. } => "recurrence_seed".to_owned(),
            KernelOriginKind::SourceArgument { index, .. } => {
                format!("source_argument_{index}")
            }
            KernelOriginKind::SourceOption { index, .. } => format!("source_option_{index}"),
        }
    )
}

fn signal_slot_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_signal_slot_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn imported_item_slot_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_import_slot_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn callable_descriptor_symbol(program: &Program, item: ItemId) -> String {
    format!(
        "aivi_{}_callable_item_{}",
        sanitize_symbol_component(program.item_name(item)),
        item.as_raw()
    )
}

fn compute_kernel_fingerprint_for(
    program: &Program,
    kernel_id: KernelId,
    kernel: &Kernel,
) -> KernelFingerprint {
    let mut hasher = FxHasher::default();
    kernel_symbol_for(program, kernel_id, kernel).hash(&mut hasher);
    format!("{kernel:?}").hash(&mut hasher);

    let mut layout_ids = BTreeSet::new();
    collect_kernel_layout_dependencies(program, kernel, &mut layout_ids);

    let mut item_ids = BTreeSet::new();
    collect_kernel_item_dependencies(kernel, &mut item_ids);
    for item_id in &item_ids {
        collect_item_layout_dependencies(program, *item_id, &mut layout_ids);
    }

    for layout_id in layout_ids {
        layout_id.hash(&mut hasher);
        format!("{:?}", program.layouts()[layout_id]).hash(&mut hasher);
    }

    for item_id in item_ids {
        let item = &program.items()[item_id];
        item_id.hash(&mut hasher);
        item.name.hash(&mut hasher);
        format!("{:?}", item.kind).hash(&mut hasher);
        item.parameters.hash(&mut hasher);
        item.body.hash(&mut hasher);

        match &item.kind {
            ItemKind::Signal(_) => signal_slot_symbol(program, item_id).hash(&mut hasher),
            _ if item.body.is_none() => {
                imported_item_slot_symbol(program, item_id).hash(&mut hasher)
            }
            _ => {}
        }

        if item.body.is_some() {
            callable_descriptor_symbol(program, item_id).hash(&mut hasher);
        }

        if let Some(body) = item.body {
            let body_kernel = &program.kernels()[body];
            kernel_symbol_for(program, body, body_kernel).hash(&mut hasher);
            format!("{:?}", body_kernel.convention).hash(&mut hasher);
        }
    }

    KernelFingerprint::new(hasher.finish())
}

fn collect_kernel_layout_dependencies(
    program: &Program,
    kernel: &Kernel,
    layout_ids: &mut BTreeSet<LayoutId>,
) {
    if let Some(input_subject) = kernel.input_subject {
        collect_layout_dependency(program, input_subject, layout_ids);
    }
    for layout in &kernel.inline_subjects {
        collect_layout_dependency(program, *layout, layout_ids);
    }
    for layout in &kernel.environment {
        collect_layout_dependency(program, *layout, layout_ids);
    }
    collect_layout_dependency(program, kernel.result_layout, layout_ids);
    for parameter in &kernel.convention.parameters {
        collect_layout_dependency(program, parameter.layout, layout_ids);
    }
    collect_layout_dependency(program, kernel.convention.result.layout, layout_ids);
    for (_, expr) in kernel.exprs().iter() {
        collect_layout_dependency(program, expr.layout, layout_ids);
    }
}

fn collect_kernel_item_dependencies(kernel: &Kernel, item_ids: &mut BTreeSet<ItemId>) {
    item_ids.extend(kernel.global_items.iter().copied());
    for (_, expr) in kernel.exprs().iter() {
        if let KernelExprKind::Item(item) = &expr.kind {
            item_ids.insert(*item);
        }
    }
}

fn collect_item_layout_dependencies(
    program: &Program,
    item_id: ItemId,
    layout_ids: &mut BTreeSet<LayoutId>,
) {
    let item = &program.items()[item_id];
    for layout in &item.parameters {
        collect_layout_dependency(program, *layout, layout_ids);
    }
    if let Some(body) = item.body {
        let kernel = &program.kernels()[body];
        for parameter in &kernel.convention.parameters {
            collect_layout_dependency(program, parameter.layout, layout_ids);
        }
        collect_layout_dependency(program, kernel.convention.result.layout, layout_ids);
    }
}

fn collect_layout_dependency(
    program: &Program,
    layout_id: LayoutId,
    layout_ids: &mut BTreeSet<LayoutId>,
) {
    if !layout_ids.insert(layout_id) {
        return;
    }

    match &program.layouts()[layout_id].kind {
        LayoutKind::Primitive(_) => {}
        LayoutKind::Tuple(elements) => {
            for element in elements {
                collect_layout_dependency(program, *element, layout_ids);
            }
        }
        LayoutKind::Record(fields) => {
            for field in fields {
                collect_layout_dependency(program, field.layout, layout_ids);
            }
        }
        LayoutKind::Sum(variants) => {
            for variant in variants {
                if let Some(payload) = variant.payload {
                    collect_layout_dependency(program, payload, layout_ids);
                }
            }
        }
        LayoutKind::Arrow { parameter, result } => {
            collect_layout_dependency(program, *parameter, layout_ids);
            collect_layout_dependency(program, *result, layout_ids);
        }
        LayoutKind::List { element }
        | LayoutKind::Set { element }
        | LayoutKind::Option { element }
        | LayoutKind::Signal { element } => {
            collect_layout_dependency(program, *element, layout_ids);
        }
        LayoutKind::Map { key, value }
        | LayoutKind::Result { error: key, value }
        | LayoutKind::Validation { error: key, value }
        | LayoutKind::Task { error: key, value } => {
            collect_layout_dependency(program, *key, layout_ids);
            collect_layout_dependency(program, *value, layout_ids);
        }
        LayoutKind::AnonymousDomain { carrier, .. } => {
            collect_layout_dependency(program, *carrier, layout_ids);
        }
        LayoutKind::Domain { arguments, .. } => {
            for argument in arguments {
                collect_layout_dependency(program, *argument, layout_ids);
            }
        }
        LayoutKind::Opaque {
            arguments,
            variants,
            ..
        } => {
            for argument in arguments {
                collect_layout_dependency(program, *argument, layout_ids);
            }
            for variant in variants {
                if let Some(payload) = variant.payload {
                    collect_layout_dependency(program, payload, layout_ids);
                }
            }
        }
    }
}

fn sanitize_symbol_component(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "item".to_owned()
    } else {
        out
    }
}

fn validate_backend_program(program: &Program) -> Result<(), CodegenErrors> {
    if let Err(errors) = validate_program(program) {
        return Err(CodegenErrors::new(
            errors
                .into_errors()
                .into_iter()
                .map(CodegenError::InvalidBackendProgram)
                .collect(),
        ));
    }
    Ok(())
}

fn wrap_one(error: CodegenError) -> CodegenErrors {
    CodegenErrors::new(vec![error])
}
// TEST_MARKER_12345
