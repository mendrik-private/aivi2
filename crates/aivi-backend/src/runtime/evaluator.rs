pub struct KernelEvaluator<'a> {
    program: &'a Program,
    item_cache: BTreeMap<ItemId, RuntimeValue>,
    item_stack: BTreeSet<ItemId>,
    /// Ordered evaluation trace: items visited during the current evaluation,
    /// in the order they were first entered. Used for error rendering.
    eval_trace: Vec<EvalFrame>,
    last_kernel_call: Option<LastKernelCall>,
    profile: Option<KernelEvaluationProfile>,
}

/// Sentinel `KernelId` used when applying a closure during task composition (map/chain/join).
/// Only used for error diagnostics — actual program kernels use arena-allocated IDs.
pub const TASK_COMPOSITION_KERNEL_ID: KernelId = KernelId::from_raw(u32::MAX);
/// Sentinel `KernelExprId` paired with [`TASK_COMPOSITION_KERNEL_ID`].
pub const TASK_COMPOSITION_EXPR_ID: KernelExprId = KernelExprId::from_raw(u32::MAX);

/// Callback interface that lets the task executor apply a user closure to a value.
///
/// The executor holds [`RuntimeTaskPlan::Map`] / [`RuntimeTaskPlan::Chain`] variants whose
/// `function` field is a [`RuntimeValue::Callable`]. Executing those variants requires
/// calling back into the Cranelift evaluator.  Callers that have a live [`KernelEvaluator`]
/// implement this trait and supply it to [`execute_runtime_task_plan_with_applier`].
pub trait TaskFunctionApplier {
    fn apply_task_function(
        &mut self,
        function: RuntimeValue,
        args: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError>;
}

impl TaskFunctionApplier for KernelEvaluator<'_> {
    fn apply_task_function(
        &mut self,
        function: RuntimeValue,
        args: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        self.apply_callable(
            TASK_COMPOSITION_KERNEL_ID,
            TASK_COMPOSITION_EXPR_ID,
            function,
            args,
            globals,
        )
    }
}

impl<'a> KernelEvaluator<'a> {
    pub fn new(program: &'a Program) -> Self {
        Self {
            program,
            item_cache: BTreeMap::new(),
            item_stack: BTreeSet::new(),
            eval_trace: Vec::new(),
            last_kernel_call: None,
            profile: None,
        }
    }

    pub fn new_profiled(program: &'a Program) -> Self {
        let mut evaluator = Self::new(program);
        evaluator.profile = Some(KernelEvaluationProfile::default());
        evaluator
    }

    pub fn program(&self) -> &'a Program {
        self.program
    }

    pub fn profile(&self) -> Option<&KernelEvaluationProfile> {
        self.profile.as_ref()
    }

    pub fn profile_snapshot(&self) -> Option<KernelEvaluationProfile> {
        self.profile.clone()
    }

    /// Return the current evaluation trace (items visited, in entry order).
    ///
    /// Useful for error rendering: call this after an evaluation error to
    /// get the chain of item evaluations that led to the failure.
    pub fn eval_trace(&self) -> &[EvalFrame] {
        &self.eval_trace
    }

    pub fn evaluate_kernel(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let (result, expected) =
            self.evaluate_kernel_raw(kernel_id, input_subject, environment, globals)?;
        if !value_matches_layout(self.program, &result, expected) {
            return Err(EvaluationError::KernelResultLayoutMismatch {
                kernel: kernel_id,
                expected,
                found: result,
            });
        }
        Ok(result)
    }

    pub fn evaluate_signal_body_kernel(
        &mut self,
        kernel_id: KernelId,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let (result, expected) = self.evaluate_kernel_raw(kernel_id, None, environment, globals)?;
        normalize_signal_kernel_result(self.program, kernel_id, result, expected)
    }

    pub fn apply_runtime_callable(
        &mut self,
        kernel_id: KernelId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        self.apply_callable(
            kernel_id,
            KernelExprId::from_raw(0),
            callee,
            arguments,
            globals,
        )
    }

    pub fn subtract_runtime_values(
        &self,
        kernel_id: KernelId,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        self.apply_binary(
            kernel_id,
            KernelExprId::from_raw(0),
            BinaryOperator::Subtract,
            left,
            right,
        )
    }

    fn evaluate_kernel_raw(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<(RuntimeValue, LayoutId), EvaluationError> {
        let started_at = self.profile.as_ref().map(|_| Instant::now());
        let kernel = self
            .program
            .kernels()
            .get(kernel_id)
            .ok_or(EvaluationError::UnknownKernel { kernel: kernel_id })?;
        match (kernel.input_subject, input_subject) {
            (Some(expected), Some(value)) => {
                if !value_matches_layout(self.program, value, expected) {
                    return Err(EvaluationError::KernelInputLayoutMismatch {
                        kernel: kernel_id,
                        expected,
                        found: value.clone(),
                    });
                }
            }
            (Some(_), None) => {
                return Err(EvaluationError::MissingInputSubject { kernel: kernel_id });
            }
            (None, Some(_)) => {
                return Err(EvaluationError::UnexpectedInputSubject { kernel: kernel_id });
            }
            (None, None) => {}
        }
        if environment.len() != kernel.environment.len() {
            return Err(EvaluationError::KernelEnvironmentCountMismatch {
                kernel: kernel_id,
                expected: kernel.environment.len(),
                found: environment.len(),
            });
        }
        for (index, (expected, value)) in kernel
            .environment
            .iter()
            .zip(environment.iter())
            .enumerate()
        {
            if !value_matches_layout(self.program, value, *expected) {
                return Err(EvaluationError::KernelEnvironmentLayoutMismatch {
                    kernel: kernel_id,
                    slot: EnvSlotId::from_raw(index as u32),
                    expected: *expected,
                    found: value.clone(),
                });
            }
        }
        // Check the single-entry call cache before doing any work.
        if let Some((cached_result, cached_layout)) =
            self.last_kernel_call.as_ref().and_then(|last| {
                (last.kernel_id == kernel_id
                    && last.input_subject.as_ref() == input_subject
                    && last.environment.as_ref() == environment)
                    .then(|| (last.result.clone(), last.result_layout))
            })
        {
            self.record_kernel_profile(
                kernel_id,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                true,
            );
            return Ok((cached_result, cached_layout));
        }
        let inline_subjects = vec![None; kernel.inline_subjects.len()];
        let result = self.evaluate_expr(
            kernel_id,
            kernel.root,
            input_subject,
            environment,
            &inline_subjects,
            globals,
        );
        self.record_kernel_profile(
            kernel_id,
            started_at.map_or(Duration::ZERO, |started| started.elapsed()),
            false,
        );
        let result = result?;
        self.last_kernel_call = Some(LastKernelCall {
            kernel_id,
            input_subject: input_subject.cloned(),
            environment: environment.to_vec().into_boxed_slice(),
            result: result.clone(),
            result_layout: kernel.result_layout,
        });
        Ok((result, kernel.result_layout))
    }

    pub fn evaluate_item(
        &mut self,
        item: ItemId,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        if let Some(value) = globals.get(&item) {
            return Ok(value.clone());
        }
        let started_at = self.profile.as_ref().map(|_| Instant::now());
        if let Some(value) = self.item_cache.get(&item).cloned() {
            self.record_item_profile(
                item,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                true,
            );
            return Ok(value);
        }
        let item_decl = self
            .program
            .items()
            .get(item)
            .ok_or(EvaluationError::UnknownItem { item })?;
        let kernel = item_decl
            .body
            .ok_or(EvaluationError::MissingItemBody { item })?;
        if !item_decl.parameters.is_empty() {
            return Ok(RuntimeValue::Callable(RuntimeCallable::ItemBody {
                item,
                kernel,
                parameters: item_decl.parameters.clone(),
                bound_arguments: Vec::new(),
            }));
        }
        if !self.item_stack.insert(item) {
            self.record_item_profile(
                item,
                started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                false,
            );
            return Err(EvaluationError::RecursiveItemEvaluation { item });
        }
        self.eval_trace.push(EvalFrame { item, kernel });
        let result = self.evaluate_kernel_raw(kernel, None, &[], globals);
        self.item_stack.remove(&item);
        let (raw_result, expected) = match result {
            Ok((v, layout)) => {
                self.eval_trace.pop();
                (v, layout)
            }
            Err(e) => {
                self.record_item_profile(
                    item,
                    started_at.map_or(Duration::ZERO, |started| started.elapsed()),
                    false,
                );
                return Err(e);
            }
        };
        let result = if matches!(item_decl.kind, crate::ItemKind::Signal(_)) {
            normalize_signal_kernel_result(self.program, kernel, raw_result, expected)?
        } else {
            if !value_matches_layout(self.program, &raw_result, expected) {
                return Err(EvaluationError::KernelResultLayoutMismatch {
                    kernel,
                    expected,
                    found: raw_result,
                });
            };
            raw_result
        };
        self.record_item_profile(
            item,
            started_at.map_or(Duration::ZERO, |started| started.elapsed()),
            false,
        );
        self.item_cache.insert(item, result.clone());
        Ok(result)
    }

    fn record_kernel_profile(&mut self, kernel: KernelId, elapsed: Duration, cache_hit: bool) {
        if let Some(profile) = &mut self.profile {
            profile.record_kernel(kernel, elapsed, cache_hit);
        }
    }

    fn record_item_profile(&mut self, item: ItemId, elapsed: Duration, cache_hit: bool) {
        if let Some(profile) = &mut self.profile {
            profile.record_item(item, elapsed, cache_hit);
        }
    }

    fn evaluate_expr(
        &mut self,
        kernel_id: KernelId,
        root: KernelExprId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        inline_subjects: &[Option<RuntimeValue>],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        enum Task {
            Visit(KernelExprId),
            BuildOptionSome,
            BuildText {
                expr: KernelExprId,
                fragments: Vec<Option<Box<str>>>,
            },
            BuildTuple {
                len: usize,
            },
            BuildList {
                len: usize,
            },
            BuildSet {
                len: usize,
            },
            BuildMap {
                len: usize,
            },
            BuildRecord {
                labels: Vec<Box<str>>,
            },
            BuildProjection {
                expr: KernelExprId,
                base: ProjectionBuild,
                path: Vec<Box<str>>,
            },
            BuildApply {
                expr: KernelExprId,
                arguments: usize,
            },
            BuildUnary {
                expr: KernelExprId,
                operator: UnaryOperator,
            },
            BuildBinary {
                expr: KernelExprId,
                operator: BinaryOperator,
            },
        }

        enum ProjectionBuild {
            Subject(SubjectRef),
            Expr,
        }

        let kernel = &self.program.kernels()[kernel_id];
        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr_id) => {
                    let expr = &kernel.exprs()[expr_id];
                    match &expr.kind {
                        KernelExprKind::Subject(subject) => values.push(self.subject_value(
                            kernel_id,
                            expr_id,
                            *subject,
                            input_subject,
                            inline_subjects,
                            globals,
                        )?),
                        KernelExprKind::OptionSome { payload } => {
                            tasks.push(Task::BuildOptionSome);
                            tasks.push(Task::Visit(*payload));
                        }
                        KernelExprKind::OptionNone => values.push(RuntimeValue::OptionNone),
                        KernelExprKind::Environment(slot) => {
                            let index = slot.as_raw() as usize;
                            let value = environment.get(index).cloned().ok_or(
                                EvaluationError::UnknownEnvironmentSlot {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    slot: *slot,
                                },
                            )?;
                            values.push(value);
                        }
                        KernelExprKind::Item(item) => {
                            let value = self.evaluate_item(*item, globals)?;
                            values.push(value);
                        }
                        KernelExprKind::SumConstructor(handle) => {
                            // Zero-arity constructors are already fully applied: emit Sum directly.
                            let value = if handle.field_count == 0 {
                                RuntimeValue::Sum(RuntimeSumValue {
                                    item: handle.item,
                                    type_name: handle.type_name.clone(),
                                    variant_name: handle.variant_name.clone(),
                                    fields: Vec::new(),
                                })
                            } else {
                                RuntimeValue::Callable(RuntimeCallable::SumConstructor {
                                    handle: handle.clone(),
                                    bound_arguments: Vec::new(),
                                })
                            };
                            values.push(value)
                        }
                        KernelExprKind::DomainMember(handle) => {
                            let (parameters, result) =
                                callable_signature(self.program, expr.layout);
                            values.push(RuntimeValue::Callable(RuntimeCallable::DomainMember {
                                handle: handle.clone(),
                                parameters,
                                result,
                                bound_arguments: Vec::new(),
                            }))
                        }
                        KernelExprKind::BuiltinClassMember(intrinsic) => {
                            values.push(runtime_class_member_value(*intrinsic))
                        }
                        KernelExprKind::Builtin(term) => values.push(map_builtin(*term)),
                        KernelExprKind::IntrinsicValue(value) => {
                            values.push(runtime_intrinsic_value(kernel_id, expr_id, *value)?)
                        }
                        KernelExprKind::Integer(integer) => {
                            let value = integer.raw.parse::<i64>().map(RuntimeValue::Int).map_err(
                                |_| EvaluationError::InvalidIntegerLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: integer.raw.clone(),
                                },
                            )?;
                            values.push(value);
                        }
                        KernelExprKind::Float(float) => {
                            let value = RuntimeFloat::parse_literal(float.raw.as_ref())
                                .map(RuntimeValue::Float)
                                .ok_or_else(|| EvaluationError::InvalidFloatLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: float.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::Decimal(decimal) => {
                            let value = RuntimeDecimal::parse_literal(decimal.raw.as_ref())
                                .map(RuntimeValue::Decimal)
                                .ok_or_else(|| EvaluationError::InvalidDecimalLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: decimal.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::BigInt(bigint) => {
                            let value = RuntimeBigInt::parse_literal(bigint.raw.as_ref())
                                .map(RuntimeValue::BigInt)
                                .ok_or_else(|| EvaluationError::InvalidBigIntLiteral {
                                    kernel: kernel_id,
                                    expr: expr_id,
                                    raw: bigint.raw.clone(),
                                })?;
                            values.push(value);
                        }
                        KernelExprKind::SuffixedInteger(integer) => {
                            values.push(RuntimeValue::SuffixedInteger {
                                raw: integer.raw.clone(),
                                suffix: integer.suffix.clone(),
                            });
                        }
                        KernelExprKind::Text(text) => {
                            tasks.push(Task::BuildText {
                                expr: expr_id,
                                fragments: text
                                    .segments
                                    .iter()
                                    .map(|segment| match segment {
                                        crate::TextSegment::Fragment { raw, .. } => {
                                            Some(raw.clone())
                                        }
                                        crate::TextSegment::Interpolation { .. } => None,
                                    })
                                    .collect(),
                            });
                            for segment in text.segments.iter().rev() {
                                if let crate::TextSegment::Interpolation { expr, .. } = segment {
                                    tasks.push(Task::Visit(*expr));
                                }
                            }
                        }
                        KernelExprKind::Tuple(elements) => {
                            tasks.push(Task::BuildTuple {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::List(elements) => {
                            tasks.push(Task::BuildList {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::Map(entries) => {
                            tasks.push(Task::BuildMap { len: entries.len() });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(entry.value));
                                tasks.push(Task::Visit(entry.key));
                            }
                        }
                        KernelExprKind::Set(elements) => {
                            tasks.push(Task::BuildSet {
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(*element));
                            }
                        }
                        KernelExprKind::Record(fields) => {
                            tasks.push(Task::BuildRecord {
                                labels: fields.iter().map(|field| field.label.clone()).collect(),
                            });
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(field.value));
                            }
                        }
                        KernelExprKind::Projection { base, path } => {
                            // Build tasks LIFO: push BuildProjection first so Visit(inner) is
                            // processed first, pushing the value that BuildProjection will pop.
                            let base_build = match base {
                                ProjectionBase::Subject(subject) => {
                                    ProjectionBuild::Subject(*subject)
                                }
                                ProjectionBase::Expr(_) => ProjectionBuild::Expr,
                            };
                            tasks.push(Task::BuildProjection {
                                expr: expr_id,
                                base: base_build,
                                path: path.clone(),
                            });
                            if let ProjectionBase::Expr(inner) = base {
                                tasks.push(Task::Visit(*inner));
                            }
                        }
                        KernelExprKind::Apply { callee, arguments } => {
                            tasks.push(Task::BuildApply {
                                expr: expr_id,
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(*argument));
                            }
                            tasks.push(Task::Visit(*callee));
                        }
                        KernelExprKind::Unary { operator, expr } => {
                            tasks.push(Task::BuildUnary {
                                expr: expr_id,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(*expr));
                        }
                        KernelExprKind::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            tasks.push(Task::BuildBinary {
                                expr: expr_id,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(*right));
                            tasks.push(Task::Visit(*left));
                        }
                        KernelExprKind::Pipe(_) => {
                            let pipe = match &expr.kind {
                                KernelExprKind::Pipe(pipe) => pipe,
                                _ => unreachable!(),
                            };
                            values.push(self.evaluate_inline_pipe(
                                kernel_id,
                                expr_id,
                                pipe,
                                input_subject,
                                environment,
                                inline_subjects,
                                globals,
                            )?);
                        }
                    }
                }
                Task::BuildOptionSome => {
                    let payload = pop_value(&mut values);
                    values.push(RuntimeValue::OptionSome(Box::new(payload)));
                }
                Task::BuildText { expr, fragments } => {
                    let mut rendered = String::new();
                    let interpolation_count = fragments
                        .iter()
                        .filter(|fragment| fragment.is_none())
                        .count();
                    let interpolations = drain_tail(&mut values, interpolation_count);
                    let mut interpolation_iter = interpolations.into_iter();
                    for fragment in fragments {
                        match fragment {
                            Some(raw) => rendered.push_str(&raw),
                            None => {
                                let value =
                                    strip_signal(interpolation_iter.next().expect(
                                        "interpolation placeholders should align with values",
                                    ));
                                if matches!(value, RuntimeValue::Callable(_)) {
                                    return Err(EvaluationError::InvalidInterpolationValue {
                                        kernel: kernel_id,
                                        expr,
                                        found: value,
                                    });
                                }
                                value
                                    .write_display_text(&mut rendered)
                                    .expect("writing into a String should not fail");
                            }
                        }
                    }
                    values.push(RuntimeValue::Text(rendered.into_boxed_str()));
                }
                Task::BuildTuple { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::Tuple(elements))
                }
                Task::BuildList { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::List(elements))
                }
                Task::BuildSet { len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(RuntimeValue::Set(elements))
                }
                Task::BuildMap { len } => {
                    let entries = drain_tail(&mut values, len * 2)
                        .chunks_exact(2)
                        .map(|pair| RuntimeMapEntry {
                            key: pair[0].clone(),
                            value: pair[1].clone(),
                        })
                        .collect();
                    values.push(RuntimeValue::Map(RuntimeMap::from_entries(entries)));
                }
                Task::BuildRecord { labels } => {
                    let len = labels.len();
                    let values_tail = drain_tail(&mut values, len);
                    values.push(RuntimeValue::Record(
                        labels
                            .into_iter()
                            .zip(values_tail.into_iter())
                            .map(|(label, value)| RuntimeRecordField { label, value })
                            .collect(),
                    ));
                }
                Task::BuildProjection { expr, base, path } => {
                    let mut value = match base {
                        ProjectionBuild::Subject(subject) => self.subject_value(
                            kernel_id,
                            expr,
                            subject,
                            input_subject,
                            inline_subjects,
                            globals,
                        )?,
                        ProjectionBuild::Expr => pop_value(&mut values),
                    };
                    for label in path {
                        value = project_field(kernel_id, expr, value, &label)?;
                    }
                    values.push(value);
                }
                Task::BuildApply { expr, arguments } => {
                    let arguments = drain_tail(&mut values, arguments);
                    let callee = pop_value(&mut values);
                    let value = self.apply_callable(kernel_id, expr, callee, arguments, globals)?;
                    values.push(value);
                }
                Task::BuildUnary { expr, operator } => {
                    let operand = pop_value(&mut values);
                    let result = self.apply_unary(kernel_id, expr, operator, operand)?;
                    values.push(result);
                }
                Task::BuildBinary { expr, operator } => {
                    let right = pop_value(&mut values);
                    let left = pop_value(&mut values);
                    let result = self.apply_binary(kernel_id, expr, operator, left, right)?;
                    values.push(result);
                }
            }
        }
        Ok(pop_value(&mut values))
    }

    fn evaluate_inline_pipe(
        &mut self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        pipe: &crate::InlinePipeExpr,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        inline_subjects: &[Option<RuntimeValue>],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let kernel = &self.program.kernels()[kernel_id];
        let mut current = self.evaluate_expr(
            kernel_id,
            pipe.head,
            input_subject,
            environment,
            inline_subjects,
            globals,
        )?;
        let mut pipe_subjects = inline_subjects.to_vec();
        for stage in &pipe.stages {
            let stage_found = current.clone();
            current = coerce_inline_pipe_value(self.program, current, stage.input_layout).ok_or(
                EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: stage.input_layout,
                    found: stage_found,
                },
            )?;
            pipe_subjects[stage.subject.index()] = Some(current.clone());
            if let Some(slot) = stage.subject_memo {
                pipe_subjects[slot.index()] = Some(current.clone());
            }
            let stage_subjects = pipe_subjects.clone();
            let result = match &stage.kind {
                InlinePipeStageKind::Transform { mode, expr } => match mode {
                    aivi_hir::PipeTransformMode::Apply | aivi_hir::PipeTransformMode::Replace => {
                        self.evaluate_expr(
                            kernel_id,
                            *expr,
                            input_subject,
                            environment,
                            &stage_subjects,
                            globals,
                        )?
                    }
                },
                InlinePipeStageKind::Tap { expr } => {
                    let _ = self.evaluate_expr(
                        kernel_id,
                        *expr,
                        input_subject,
                        environment,
                        &stage_subjects,
                        globals,
                    )?;
                    current
                }
                InlinePipeStageKind::Debug { label } => {
                    eprintln!("{label}: {current}");
                    current
                }
                InlinePipeStageKind::Gate { predicate, .. } => {
                    let result = self.evaluate_expr(
                        kernel_id,
                        *predicate,
                        input_subject,
                        environment,
                        &stage_subjects,
                        globals,
                    )?;
                    match strip_signal(result) {
                        RuntimeValue::Bool(true) => RuntimeValue::OptionSome(Box::new(current)),
                        RuntimeValue::Bool(false) => RuntimeValue::OptionNone,
                        _ => {
                            return Err(EvaluationError::UnsupportedInlinePipePattern {
                                kernel: kernel_id,
                                expr: expr_id,
                            });
                        }
                    }
                }
                InlinePipeStageKind::Case { arms } => {
                    let mut matched = None;
                    for arm in arms {
                        let mut branch_subjects = stage_subjects.clone();
                        if self.match_inline_pipe_pattern(
                            kernel_id,
                            expr_id,
                            kernel,
                            &arm.pattern,
                            &current,
                            &mut branch_subjects,
                        )? {
                            matched = Some(self.evaluate_expr(
                                kernel_id,
                                arm.body,
                                input_subject,
                                environment,
                                &branch_subjects,
                                globals,
                            )?);
                            break;
                        }
                    }
                    matched.ok_or_else(|| EvaluationError::InlinePipeCaseNoMatch {
                        kernel: kernel_id,
                        expr: expr_id,
                        subject: current.clone(),
                    })?
                }
                InlinePipeStageKind::TruthyFalsy { truthy, falsy } => {
                    let (branch, payload) = self
                        .select_truthy_falsy_branch(kernel_id, expr_id, &current, truthy, falsy)?;
                    let mut branch_subjects = stage_subjects;
                    if let (Some(slot), Some(payload)) = (branch.payload_subject, payload) {
                        branch_subjects[slot.index()] = Some(payload);
                    }
                    self.evaluate_expr(
                        kernel_id,
                        branch.body,
                        input_subject,
                        environment,
                        &branch_subjects,
                        globals,
                    )?
                }
                InlinePipeStageKind::FanOut { map_expr } => {
                    let elements = match current {
                        RuntimeValue::List(ref items) => items.clone(),
                        _ => {
                            return Err(EvaluationError::UnsupportedInlinePipePattern {
                                kernel: kernel_id,
                                expr: expr_id,
                            });
                        }
                    };
                    let mapped = elements
                        .iter()
                        .map(|element| {
                            let mut element_subjects = stage_subjects.clone();
                            element_subjects[stage.subject.index()] = Some(element.clone());
                            self.evaluate_expr(
                                kernel_id,
                                *map_expr,
                                input_subject,
                                environment,
                                &element_subjects,
                                globals,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    RuntimeValue::List(mapped)
                }
            };
            let result_found = result.clone();
            current = coerce_inline_pipe_value(self.program, result, stage.result_layout).ok_or(
                EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: stage.result_layout,
                    found: result_found,
                },
            )?;
            if let Some(slot) = stage.result_memo {
                pipe_subjects[slot.index()] = Some(current.clone());
            }
        }
        Ok(current)
    }

    fn select_truthy_falsy_branch<'b>(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        value: &RuntimeValue,
        truthy: &'b crate::InlinePipeTruthyFalsyBranch,
        falsy: &'b crate::InlinePipeTruthyFalsyBranch,
    ) -> Result<(&'b crate::InlinePipeTruthyFalsyBranch, Option<RuntimeValue>), EvaluationError>
    {
        if let Some(payload) = truthy_falsy_payload(value, truthy.constructor) {
            return Ok((truthy, payload));
        }
        if let Some(payload) = truthy_falsy_payload(value, falsy.constructor) {
            return Ok((falsy, payload));
        }
        Err(EvaluationError::UnsupportedInlinePipePattern {
            kernel: kernel_id,
            expr: expr_id,
        })
    }

    fn match_inline_pipe_pattern(
        &self,
        kernel_id: KernelId,
        expr_id: KernelExprId,
        kernel: &crate::Kernel,
        pattern: &InlinePipePattern,
        value: &RuntimeValue,
        inline_subjects: &mut [Option<RuntimeValue>],
    ) -> Result<bool, EvaluationError> {
        match &pattern.kind {
            InlinePipePatternKind::Wildcard => Ok(true),
            InlinePipePatternKind::Binding { subject } => {
                let expected = kernel.inline_subjects.get(subject.index()).copied().ok_or(
                    EvaluationError::UnknownInlineSubject {
                        kernel: kernel_id,
                        expr: expr_id,
                        slot: *subject,
                    },
                )?;
                if !value_matches_layout(self.program, value, expected) {
                    return Err(EvaluationError::UnsupportedInlinePipePattern {
                        kernel: kernel_id,
                        expr: expr_id,
                    });
                }
                inline_subjects[subject.index()] = Some(value.clone());
                Ok(true)
            }
            InlinePipePatternKind::Integer(integer) => Ok(matches!(
                value,
                RuntimeValue::Int(found) if integer.raw.parse::<i64>().ok() == Some(*found)
            )),
            InlinePipePatternKind::Text(raw) => {
                Ok(matches!(value, RuntimeValue::Text(found) if found.as_ref() == raw.as_ref()))
            }
            InlinePipePatternKind::Tuple(elements) => {
                let RuntimeValue::Tuple(values) = value else {
                    return Ok(false);
                };
                if values.len() != elements.len() {
                    return Ok(false);
                }
                for (pattern, value) in elements.iter().zip(values.iter()) {
                    if !self.match_inline_pipe_pattern(
                        kernel_id,
                        expr_id,
                        kernel,
                        pattern,
                        value,
                        inline_subjects,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            InlinePipePatternKind::List { elements, rest } => {
                let RuntimeValue::List(values) = value else {
                    return Ok(false);
                };
                if values.len() < elements.len() {
                    return Ok(false);
                }
                if rest.is_none() && values.len() != elements.len() {
                    return Ok(false);
                }
                for (pattern, value) in elements.iter().zip(values.iter()) {
                    if !self.match_inline_pipe_pattern(
                        kernel_id,
                        expr_id,
                        kernel,
                        pattern,
                        value,
                        inline_subjects,
                    )? {
                        return Ok(false);
                    }
                }
                if let Some(rest) = rest {
                    let remaining = RuntimeValue::List(values[elements.len()..].to_vec());
                    if !self.match_inline_pipe_pattern(
                        kernel_id,
                        expr_id,
                        kernel,
                        rest,
                        &remaining,
                        inline_subjects,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            InlinePipePatternKind::Record(fields) => {
                let RuntimeValue::Record(values) = value else {
                    return Ok(false);
                };
                for field in fields {
                    let Some(value) = values
                        .iter()
                        .find(|candidate| candidate.label.as_ref() == field.label.as_ref())
                    else {
                        return Ok(false);
                    };
                    if !self.match_inline_pipe_pattern(
                        kernel_id,
                        expr_id,
                        kernel,
                        &field.pattern,
                        &value.value,
                        inline_subjects,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            InlinePipePatternKind::Constructor {
                constructor,
                arguments,
            } => match constructor {
                InlinePipeConstructor::Builtin(constructor) => {
                    let Some(payload) = truthy_falsy_payload(value, *constructor) else {
                        return Ok(false);
                    };
                    match (payload, arguments.as_slice()) {
                        (None, []) => Ok(true),
                        (Some(payload), [argument]) => self.match_inline_pipe_pattern(
                            kernel_id,
                            expr_id,
                            kernel,
                            argument,
                            &payload,
                            inline_subjects,
                        ),
                        _ => Err(EvaluationError::UnsupportedInlinePipePattern {
                            kernel: kernel_id,
                            expr: expr_id,
                        }),
                    }
                }
                InlinePipeConstructor::Sum(handle) => {
                    let RuntimeValue::Sum(value) = value else {
                        return Ok(false);
                    };
                    if value.item != handle.item
                        || value.variant_name.as_ref() != handle.variant_name.as_ref()
                        || value.fields.len() != arguments.len()
                    {
                        return Ok(false);
                    }
                    for (argument, field) in arguments.iter().zip(value.fields.iter()) {
                        if !self.match_inline_pipe_pattern(
                            kernel_id,
                            expr_id,
                            kernel,
                            argument,
                            field,
                            inline_subjects,
                        )? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            },
        }
    }

    fn subject_value(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        subject: SubjectRef,
        input_subject: Option<&RuntimeValue>,
        inline_subjects: &[Option<RuntimeValue>],
        _globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match subject {
            SubjectRef::Input => input_subject
                .cloned()
                .ok_or(EvaluationError::MissingInputSubject { kernel: kernel_id }),
            SubjectRef::Inline(slot) => inline_subjects
                .get(slot.as_raw() as usize)
                .and_then(|value| value.clone())
                .ok_or(EvaluationError::UnknownInlineSubject {
                    kernel: kernel_id,
                    expr,
                    slot,
                }),
        }
    }

    fn apply_callable(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        callee: RuntimeValue,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let callee = strip_signal(callee);
        let RuntimeValue::Callable(callable) = callee else {
            return Err(EvaluationError::InvalidCallee {
                kernel: kernel_id,
                expr,
                found: callee,
            });
        };
        match callable {
            RuntimeCallable::ItemBody {
                item,
                kernel,
                parameters,
                mut bound_arguments,
            } => {
                let mut remaining_arguments = Vec::new();
                for argument in arguments {
                    if let Some(expected) = parameters.get(bound_arguments.len()).copied() {
                        let argument = coerce_runtime_value(self.program, argument, expected)
                            .unwrap_or_else(|value| value);
                        bound_arguments.push(argument);
                    } else {
                        remaining_arguments.push(argument);
                    }
                }
                if bound_arguments.len() < parameters.len() {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::ItemBody {
                        item,
                        kernel,
                        parameters,
                        bound_arguments,
                    }));
                }
                let mut remaining = bound_arguments.split_off(parameters.len());
                remaining.extend(remaining_arguments);
                let result = self.evaluate_kernel(kernel, None, &bound_arguments, globals)?;
                if remaining.is_empty() {
                    Ok(result)
                } else {
                    self.apply_callable(kernel_id, expr, result, remaining, globals)
                }
            }
            RuntimeCallable::BuiltinConstructor {
                constructor,
                mut bound_arguments,
            } => {
                bound_arguments.extend(arguments.into_iter().map(strip_signal));
                if bound_arguments.is_empty() {
                    return Ok(RuntimeValue::Callable(
                        RuntimeCallable::BuiltinConstructor {
                            constructor,
                            bound_arguments,
                        },
                    ));
                }
                let mut remaining = bound_arguments;
                let payload = remaining.remove(0);
                let value = match constructor {
                    RuntimeConstructor::Some => RuntimeValue::OptionSome(Box::new(payload)),
                    RuntimeConstructor::Ok => RuntimeValue::ResultOk(Box::new(payload)),
                    RuntimeConstructor::Err => RuntimeValue::ResultErr(Box::new(payload)),
                    RuntimeConstructor::Valid => RuntimeValue::ValidationValid(Box::new(payload)),
                    RuntimeConstructor::Invalid => {
                        RuntimeValue::ValidationInvalid(Box::new(payload))
                    }
                };
                if remaining.is_empty() {
                    Ok(value)
                } else {
                    self.apply_callable(kernel_id, expr, value, remaining, globals)
                }
            }
            RuntimeCallable::SumConstructor {
                handle,
                mut bound_arguments,
            } => {
                bound_arguments.extend(arguments.into_iter().map(strip_signal));
                if bound_arguments.len() < handle.field_count as usize {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::SumConstructor {
                        handle,
                        bound_arguments,
                    }));
                }
                let remaining = bound_arguments.split_off(handle.field_count as usize);
                let value = RuntimeValue::Sum(RuntimeSumValue {
                    item: handle.item,
                    type_name: handle.type_name.clone(),
                    variant_name: handle.variant_name.clone(),
                    fields: bound_arguments,
                });
                if remaining.is_empty() {
                    Ok(value)
                } else {
                    self.apply_callable(kernel_id, expr, value, remaining, globals)
                }
            }
            RuntimeCallable::DomainMember {
                handle,
                parameters,
                result,
                bound_arguments,
            } => {
                let mut bound_arguments = bound_arguments;
                bound_arguments.extend(arguments.into_iter().map(strip_signal));
                if bound_arguments.len() < parameters.len() {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::DomainMember {
                        handle,
                        parameters,
                        result,
                        bound_arguments,
                    }));
                }
                let remaining = bound_arguments.split_off(parameters.len());
                let value = self.evaluate_domain_member(
                    kernel_id,
                    expr,
                    &handle,
                    &parameters,
                    result,
                    bound_arguments,
                )?;
                if remaining.is_empty() {
                    Ok(value)
                } else {
                    self.apply_callable(kernel_id, expr, value, remaining, globals)
                }
            }
            RuntimeCallable::BuiltinClassMember {
                intrinsic,
                mut bound_arguments,
            } => {
                bound_arguments.extend(arguments);
                let arity = builtin_class_member_arity(intrinsic);
                if bound_arguments.len() < arity {
                    return Ok(RuntimeValue::Callable(
                        RuntimeCallable::BuiltinClassMember {
                            intrinsic,
                            bound_arguments,
                        },
                    ));
                }
                let remaining = bound_arguments.split_off(arity);
                let value = self.evaluate_builtin_class_member(
                    kernel_id,
                    expr,
                    intrinsic,
                    bound_arguments,
                    globals,
                )?;
                if remaining.is_empty() {
                    Ok(value)
                } else {
                    self.apply_callable(kernel_id, expr, value, remaining, globals)
                }
            }
            RuntimeCallable::IntrinsicValue {
                value,
                mut bound_arguments,
            } => {
                bound_arguments.extend(arguments.into_iter().map(strip_signal));
                let arity = intrinsic_value_arity(value);
                if bound_arguments.len() < arity {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::IntrinsicValue {
                        value,
                        bound_arguments,
                    }));
                }
                let remaining = bound_arguments.split_off(arity);
                let value = evaluate_intrinsic_value(kernel_id, expr, value, bound_arguments)?;
                if remaining.is_empty() {
                    Ok(value)
                } else {
                    self.apply_callable(kernel_id, expr, value, remaining, globals)
                }
            }
        }
    }

    fn evaluate_domain_member(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        handle: &DomainMemberHandle,
        parameters: &[LayoutId],
        result_layout: LayoutId,
        arguments: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        if let Some(operator) = domain_member_binary_operator(handle.member_name.as_ref()) {
            return self.evaluate_domain_binary_member(
                kernel_id,
                expr,
                handle,
                operator,
                result_layout,
                arguments,
            );
        }

        match (handle.member_name.as_ref(), arguments.as_slice()) {
            ("value" | "carrier", [argument])
                if parameters.len() == 1 && is_named_domain_layout(self.program, parameters[0]) =>
            {
                return Ok(domain_member_carrier_value(argument.clone()));
            }
            ("singleton", [argument])
                if parameters.len() == 1 && is_named_domain_layout(self.program, result_layout) =>
            {
                return Ok(RuntimeValue::List(vec![strip_signal(argument.clone())]));
            }
            ("head", [argument])
                if parameters.len() == 1 && is_named_domain_layout(self.program, parameters[0]) =>
            {
                return match strip_signal(argument.clone()) {
                    RuntimeValue::List(values) => values.into_iter().next().ok_or_else(|| {
                        EvaluationError::UnsupportedDomainMemberCall {
                            kernel: kernel_id,
                            expr,
                            handle: handle.clone(),
                        }
                    }),
                    _ => Err(EvaluationError::UnsupportedDomainMemberCall {
                        kernel: kernel_id,
                        expr,
                        handle: handle.clone(),
                    }),
                };
            }
            ("tail", [argument])
                if parameters.len() == 1 && is_named_domain_layout(self.program, parameters[0]) =>
            {
                return match strip_signal(argument.clone()) {
                    RuntimeValue::List(values) if !values.is_empty() => {
                        Ok(RuntimeValue::List(values[1..].to_vec()))
                    }
                    _ => Err(EvaluationError::UnsupportedDomainMemberCall {
                        kernel: kernel_id,
                        expr,
                        handle: handle.clone(),
                    }),
                };
            }
            ("fromList", [argument])
                if parameters.len() == 1
                    && matches!(
                        self.program.layouts().get(result_layout).map(|layout| &layout.kind),
                        Some(LayoutKind::Option { element })
                            if is_named_domain_layout(self.program, *element)
                    ) =>
            {
                return match strip_signal(argument.clone()) {
                    RuntimeValue::List(values) if values.is_empty() => Ok(RuntimeValue::OptionNone),
                    RuntimeValue::List(values) => Ok(RuntimeValue::OptionSome(Box::new(
                        RuntimeValue::List(values),
                    ))),
                    _ => Err(EvaluationError::UnsupportedDomainMemberCall {
                        kernel: kernel_id,
                        expr,
                        handle: handle.clone(),
                    }),
                };
            }
            _ => {}
        }

        if parameters.len() == 1
            && matches!(arguments.as_slice(), [_])
            && is_named_domain_layout(self.program, result_layout)
        {
            return Ok(strip_signal(
                arguments
                    .into_iter()
                    .next()
                    .expect("single-argument domain member should keep its argument"),
            ));
        }

        Err(EvaluationError::UnsupportedDomainMemberCall {
            kernel: kernel_id,
            expr,
            handle: handle.clone(),
        })
    }

    fn evaluate_domain_binary_member(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        handle: &DomainMemberHandle,
        operator: BinaryOperator,
        result_layout: LayoutId,
        arguments: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let [left, right] = arguments.as_slice() else {
            return Err(EvaluationError::UnsupportedDomainMemberCall {
                kernel: kernel_id,
                expr,
                handle: handle.clone(),
            });
        };
        let preserved_suffix = shared_suffixed_integer_suffix(left, right).ok_or_else(|| {
            EvaluationError::UnsupportedDomainMemberCall {
                kernel: kernel_id,
                expr,
                handle: handle.clone(),
            }
        })?;
        let left = coerce_domain_numeric_value(left.clone()).ok_or_else(|| {
            EvaluationError::UnsupportedDomainMemberCall {
                kernel: kernel_id,
                expr,
                handle: handle.clone(),
            }
        })?;
        let right = coerce_domain_numeric_value(right.clone()).ok_or_else(|| {
            EvaluationError::UnsupportedDomainMemberCall {
                kernel: kernel_id,
                expr,
                handle: handle.clone(),
            }
        })?;
        let value = self.apply_binary(kernel_id, expr, operator, left, right)?;
        if !matches!(
            operator,
            BinaryOperator::Add
                | BinaryOperator::Subtract
                | BinaryOperator::Multiply
                | BinaryOperator::Divide
                | BinaryOperator::Modulo
        ) || !is_named_domain_layout(self.program, result_layout)
        {
            return Ok(value);
        }
        match (value, preserved_suffix) {
            (RuntimeValue::Int(raw), Some(suffix)) => Ok(RuntimeValue::SuffixedInteger {
                raw: raw.to_string().into_boxed_str(),
                suffix,
            }),
            (value, _) => Ok(value),
        }
    }

    fn evaluate_builtin_class_member(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        arguments: Vec<RuntimeValue>,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match intrinsic {
            BuiltinClassMemberIntrinsic::StructuralEq => {
                let [left, right] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                Ok(RuntimeValue::Bool(structural_eq(
                    kernel_id, expr, &left, &right,
                )?))
            }
            BuiltinClassMemberIntrinsic::Compare {
                subject,
                ordering_item,
            } => {
                let [left, right] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.compare_builtin_subject(kernel_id, expr, subject, ordering_item, left, right)
            }
            BuiltinClassMemberIntrinsic::Append(carrier) => {
                let [left, right] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.append_builtin_carrier(kernel_id, expr, intrinsic, carrier, left, right)
            }
            BuiltinClassMemberIntrinsic::Empty(carrier) => Ok(match carrier {
                BuiltinAppendCarrier::Text => RuntimeValue::Text("".into()),
                BuiltinAppendCarrier::List => RuntimeValue::List(Vec::new()),
            }),
            BuiltinClassMemberIntrinsic::Map(carrier) => {
                let [function, subject] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.map_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, function, subject, globals,
                )
            }
            BuiltinClassMemberIntrinsic::Bimap(carrier) => {
                let [left, right, subject] = expect_arity::<3>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.bimap_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, left, right, subject, globals,
                )
            }
            BuiltinClassMemberIntrinsic::Pure(carrier) => {
                let [payload] = expect_arity::<1>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                Ok(pure_applicative_value(carrier, payload))
            }
            BuiltinClassMemberIntrinsic::Apply(carrier) => {
                let [functions, values] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.apply_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, functions, values, globals,
                )
            }
            BuiltinClassMemberIntrinsic::Chain(carrier) => {
                let [function, subject] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.chain_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, function, subject, globals,
                )
            }
            BuiltinClassMemberIntrinsic::Join(carrier) => {
                let [subject] = expect_arity::<1>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.join_builtin_carrier(kernel_id, expr, intrinsic, carrier, subject)
            }
            BuiltinClassMemberIntrinsic::Reduce(carrier) => {
                let [function, initial, subject] =
                    expect_arity::<3>(arguments).map_err(|reason| {
                        EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason,
                        }
                    })?;
                self.reduce_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, function, initial, subject, globals,
                )
            }
            BuiltinClassMemberIntrinsic::Traverse {
                traversable,
                applicative,
            } => {
                let [function, subject] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.traverse_builtin_carrier(
                    kernel_id,
                    expr,
                    intrinsic,
                    traversable,
                    applicative,
                    function,
                    subject,
                    globals,
                )
            }
            BuiltinClassMemberIntrinsic::FilterMap(carrier) => {
                let [function, subject] = expect_arity::<2>(arguments).map_err(|reason| {
                    EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason,
                    }
                })?;
                self.filter_map_builtin_carrier(
                    kernel_id, expr, intrinsic, carrier, function, subject, globals,
                )
            }
        }
    }

    fn compare_builtin_subject(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        subject: BuiltinOrdSubject,
        ordering_item: HirItemId,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        let ordering = match (subject, strip_signal(left), strip_signal(right)) {
            (BuiltinOrdSubject::Int, RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                left.cmp(&right)
            }
            (BuiltinOrdSubject::Float, RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                left.partial_cmp(&right)
                    .expect("runtime floats are finite and always comparable")
            }
            (
                BuiltinOrdSubject::Decimal,
                RuntimeValue::Decimal(left),
                RuntimeValue::Decimal(right),
            ) => left.cmp(&right),
            (
                BuiltinOrdSubject::BigInt,
                RuntimeValue::BigInt(left),
                RuntimeValue::BigInt(right),
            ) => left.cmp(&right),
            (BuiltinOrdSubject::Bool, RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => {
                left.cmp(&right)
            }
            (BuiltinOrdSubject::Text, RuntimeValue::Text(left), RuntimeValue::Text(right)) => {
                left.as_ref().cmp(right.as_ref())
            }
            (BuiltinOrdSubject::Ordering, RuntimeValue::Sum(left), RuntimeValue::Sum(right))
                if left.type_name.as_ref() == "Ordering"
                    && right.type_name.as_ref() == "Ordering" =>
            {
                ordering_rank(&left.variant_name).cmp(&ordering_rank(&right.variant_name))
            }
            _ => {
                return Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic: BuiltinClassMemberIntrinsic::Compare {
                        subject,
                        ordering_item,
                    },
                    reason: "compare received values outside the supported runtime carriers",
                });
            }
        };
        Ok(ordering_value(ordering_item, ordering))
    }

    fn append_builtin_carrier(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinAppendCarrier,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        match (carrier, strip_signal(left), strip_signal(right)) {
            (BuiltinAppendCarrier::Text, RuntimeValue::Text(left), RuntimeValue::Text(right)) => {
                Ok(RuntimeValue::Text(
                    format!("{}{}", left.as_ref(), right.as_ref()).into_boxed_str(),
                ))
            }
            (
                BuiltinAppendCarrier::List,
                RuntimeValue::List(mut left),
                RuntimeValue::List(right),
            ) => {
                left.extend(right);
                Ok(RuntimeValue::List(left))
            }
            _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                kernel: kernel_id,
                expr,
                intrinsic,
                reason: "append received values outside the supported runtime carriers",
            }),
        }
    }

    fn map_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinFunctorCarrier,
        function: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinFunctorCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut mapped = Vec::with_capacity(values.len());
                    for value in values {
                        mapped.push(self.apply_callable(
                            kernel_id,
                            expr,
                            function.clone(),
                            vec![value],
                            globals,
                        )?);
                    }
                    Ok(RuntimeValue::List(mapped))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received values outside the supported runtime carriers",
                }),
            },
            BuiltinFunctorCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(Box::new(
                    self.apply_callable(kernel_id, expr, function, vec![*value], globals)?,
                ))),
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received values outside the supported runtime carriers",
                }),
            },
            BuiltinFunctorCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
                RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(Box::new(
                    self.apply_callable(kernel_id, expr, function, vec![*value], globals)?,
                ))),
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received values outside the supported runtime carriers",
                }),
            },
            BuiltinFunctorCarrier::Validation => match strip_signal(subject) {
                RuntimeValue::ValidationInvalid(error) => {
                    Ok(RuntimeValue::ValidationInvalid(error))
                }
                RuntimeValue::ValidationValid(value) => {
                    Ok(RuntimeValue::ValidationValid(Box::new(
                        self.apply_callable(kernel_id, expr, function, vec![*value], globals)?,
                    )))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received values outside the supported runtime carriers",
                }),
            },
            BuiltinFunctorCarrier::Signal => match subject {
                RuntimeValue::Signal(value) => Ok(RuntimeValue::Signal(Box::new(
                    self.apply_callable(kernel_id, expr, function, vec![*value], globals)?,
                ))),
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received values outside the supported runtime carriers",
                }),
            },
            BuiltinFunctorCarrier::Task => match strip_signal(subject) {
                RuntimeValue::Task(plan) => match plan {
                    // Pure tasks: apply eagerly (no deferred plan needed).
                    RuntimeTaskPlan::Pure { value } => {
                        let mapped =
                            self.apply_callable(kernel_id, expr, function, vec![*value], globals)?;
                        Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                            value: Box::new(mapped),
                        }))
                    }
                    // Non-pure tasks: emit a deferred Map plan executed by the task worker.
                    other => Ok(RuntimeValue::Task(RuntimeTaskPlan::Map {
                        function: Box::new(function),
                        inner: Box::new(other),
                    })),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "map received a non-Task value for Task carrier",
                }),
            },
        }
    }

    fn bimap_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinBifunctorCarrier,
        left_function: RuntimeValue,
        right_function: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinBifunctorCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(Box::new(
                    self.apply_callable(kernel_id, expr, left_function, vec![*error], globals)?,
                ))),
                RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(Box::new(
                    self.apply_callable(kernel_id, expr, right_function, vec![*value], globals)?,
                ))),
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "bimap received values outside the supported runtime carriers",
                }),
            },
            BuiltinBifunctorCarrier::Validation => match strip_signal(subject) {
                RuntimeValue::ValidationInvalid(error) => {
                    Ok(RuntimeValue::ValidationInvalid(Box::new(
                        self.apply_callable(kernel_id, expr, left_function, vec![*error], globals)?,
                    )))
                }
                RuntimeValue::ValidationValid(value) => Ok(RuntimeValue::ValidationValid(
                    Box::new(self.apply_callable(
                        kernel_id,
                        expr,
                        right_function,
                        vec![*value],
                        globals,
                    )?),
                )),
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "bimap received values outside the supported runtime carriers",
                }),
            },
        }
    }

    fn apply_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinApplyCarrier,
        functions: RuntimeValue,
        values: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinApplyCarrier::List => match (strip_signal(functions), strip_signal(values)) {
                (RuntimeValue::List(functions), RuntimeValue::List(values)) => {
                    let mut results = Vec::new();
                    for function in functions {
                        for value in &values {
                            results.push(self.apply_callable(
                                kernel_id,
                                expr,
                                function.clone(),
                                vec![value.clone()],
                                globals,
                            )?);
                        }
                    }
                    Ok(RuntimeValue::List(results))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "apply received values outside the supported runtime carriers",
                }),
            },
            BuiltinApplyCarrier::Option => match (strip_signal(functions), strip_signal(values)) {
                (RuntimeValue::OptionSome(function), RuntimeValue::OptionSome(value)) => {
                    Ok(RuntimeValue::OptionSome(Box::new(self.apply_callable(
                        kernel_id,
                        expr,
                        *function,
                        vec![*value],
                        globals,
                    )?)))
                }
                (RuntimeValue::OptionNone, _) | (_, RuntimeValue::OptionNone) => {
                    Ok(RuntimeValue::OptionNone)
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "apply received values outside the supported runtime carriers",
                }),
            },
            BuiltinApplyCarrier::Result => match (strip_signal(functions), strip_signal(values)) {
                (RuntimeValue::ResultErr(error), _) => Ok(RuntimeValue::ResultErr(error)),
                (_, RuntimeValue::ResultErr(error)) => Ok(RuntimeValue::ResultErr(error)),
                (RuntimeValue::ResultOk(function), RuntimeValue::ResultOk(value)) => {
                    Ok(RuntimeValue::ResultOk(Box::new(self.apply_callable(
                        kernel_id,
                        expr,
                        *function,
                        vec![*value],
                        globals,
                    )?)))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "apply received values outside the supported runtime carriers",
                }),
            },
            BuiltinApplyCarrier::Validation => {
                match (strip_signal(functions), strip_signal(values)) {
                    (
                        RuntimeValue::ValidationInvalid(left),
                        RuntimeValue::ValidationInvalid(right),
                    ) => Ok(RuntimeValue::ValidationInvalid(Box::new(
                        append_validation_errors(*left, *right).map_err(|reason| {
                            EvaluationError::UnsupportedBuiltinClassMember {
                                kernel: kernel_id,
                                expr,
                                intrinsic,
                                reason,
                            }
                        })?,
                    ))),
                    (RuntimeValue::ValidationInvalid(error), _) => {
                        Ok(RuntimeValue::ValidationInvalid(error))
                    }
                    (_, RuntimeValue::ValidationInvalid(error)) => {
                        Ok(RuntimeValue::ValidationInvalid(error))
                    }
                    (
                        RuntimeValue::ValidationValid(function),
                        RuntimeValue::ValidationValid(value),
                    ) => Ok(RuntimeValue::ValidationValid(Box::new(
                        self.apply_callable(kernel_id, expr, *function, vec![*value], globals)?,
                    ))),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "apply received values outside the supported runtime carriers",
                    }),
                }
            }
            BuiltinApplyCarrier::Signal => match (functions, values) {
                (RuntimeValue::Signal(function), RuntimeValue::Signal(value)) => {
                    Ok(RuntimeValue::Signal(Box::new(self.apply_callable(
                        kernel_id,
                        expr,
                        *function,
                        vec![*value],
                        globals,
                    )?)))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "apply received values outside the supported runtime carriers",
                }),
            },
            BuiltinApplyCarrier::Task => {
                match (strip_signal(functions), strip_signal(values)) {
                    (RuntimeValue::Task(function_plan), RuntimeValue::Task(value_plan)) => {
                        match (function_plan, value_plan) {
                            // Both Pure: apply eagerly.
                            (
                                RuntimeTaskPlan::Pure { value: function },
                                RuntimeTaskPlan::Pure { value },
                            ) => {
                                let result = self.apply_callable(
                                    kernel_id,
                                    expr,
                                    *function,
                                    vec![*value],
                                    globals,
                                )?;
                                Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                                    value: Box::new(result),
                                }))
                            }
                            // Non-pure: emit a deferred Apply plan.
                            (function_plan, value_plan) => {
                                Ok(RuntimeValue::Task(RuntimeTaskPlan::Apply {
                                    function_task: Box::new(function_plan),
                                    value_task: Box::new(value_plan),
                                }))
                            }
                        }
                    }
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "apply received non-Task values for Task carrier",
                    }),
                }
            }
        }
    }

    fn chain_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinMonadCarrier,
        function: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinMonadCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut chained = Vec::new();
                    for value in values {
                        match strip_signal(self.apply_callable(
                            kernel_id,
                            expr,
                            function.clone(),
                            vec![value],
                            globals,
                        )?) {
                            RuntimeValue::List(next) => chained.extend(next),
                            _ => {
                                return Err(EvaluationError::UnsupportedBuiltinClassMember {
                                    kernel: kernel_id,
                                    expr,
                                    intrinsic,
                                    reason: "chain expected the callback to return List values",
                                });
                            }
                        }
                    }
                    Ok(RuntimeValue::List(chained))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "chain received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                RuntimeValue::OptionSome(value) => match strip_signal(self.apply_callable(
                    kernel_id,
                    expr,
                    function,
                    vec![*value],
                    globals,
                )?) {
                    RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                    RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(value)),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "chain expected the callback to return Option values",
                    }),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "chain received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
                RuntimeValue::ResultOk(value) => match strip_signal(self.apply_callable(
                    kernel_id,
                    expr,
                    function,
                    vec![*value],
                    globals,
                )?) {
                    RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(value)),
                    RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "chain expected the callback to return Result values",
                    }),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "chain received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Task => match strip_signal(subject) {
                RuntimeValue::Task(plan) => match plan {
                    // Pure inner: apply eagerly and return the resulting Task.
                    RuntimeTaskPlan::Pure { value } => {
                        match strip_signal(self.apply_callable(
                            kernel_id,
                            expr,
                            function,
                            vec![*value],
                            globals,
                        )?) {
                            RuntimeValue::Task(result_plan) => Ok(RuntimeValue::Task(result_plan)),
                            _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                                kernel: kernel_id,
                                expr,
                                intrinsic,
                                reason: "chain expected the callback to return a Task value",
                            }),
                        }
                    }
                    // Non-pure inner: emit a deferred Chain plan.
                    other => Ok(RuntimeValue::Task(RuntimeTaskPlan::Chain {
                        function: Box::new(function),
                        inner: Box::new(other),
                    })),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "chain received a non-Task value for Task carrier",
                }),
            },
        }
    }

    fn join_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinMonadCarrier,
        subject: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinMonadCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut joined = Vec::new();
                    for value in values {
                        match strip_signal(value) {
                            RuntimeValue::List(inner) => joined.extend(inner),
                            _ => {
                                return Err(EvaluationError::UnsupportedBuiltinClassMember {
                                    kernel: kernel_id,
                                    expr,
                                    intrinsic,
                                    reason: "join expected List (List A) values",
                                });
                            }
                        }
                    }
                    Ok(RuntimeValue::List(joined))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "join received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                RuntimeValue::OptionSome(value) => match strip_signal(*value) {
                    RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                    RuntimeValue::OptionSome(inner) => Ok(RuntimeValue::OptionSome(inner)),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "join expected Option (Option A) values",
                    }),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "join received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
                RuntimeValue::ResultOk(value) => match strip_signal(*value) {
                    RuntimeValue::ResultOk(inner) => Ok(RuntimeValue::ResultOk(inner)),
                    RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "join expected Result E (Result E A) values",
                    }),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "join received values outside the supported runtime carriers",
                }),
            },
            BuiltinMonadCarrier::Task => match strip_signal(subject) {
                RuntimeValue::Task(outer_plan) => match outer_plan {
                    // Pure outer: the inner value must itself be a Task — return it directly.
                    RuntimeTaskPlan::Pure { value } => match strip_signal(*value) {
                        RuntimeValue::Task(inner_plan) => Ok(RuntimeValue::Task(inner_plan)),
                        _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason: "join expected Task E (Task E A) — inner value was not a Task",
                        }),
                    },
                    // Non-pure outer: emit a deferred Join plan.
                    other => Ok(RuntimeValue::Task(RuntimeTaskPlan::Join {
                        outer: Box::new(other),
                    })),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "join received a non-Task value for Task carrier",
                }),
            },
        }
    }

    fn reduce_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinFoldableCarrier,
        function: RuntimeValue,
        initial: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        let initial = strip_signal(initial);
        match carrier {
            BuiltinFoldableCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut accumulator = initial;
                    for value in values {
                        accumulator = self.apply_callable(
                            kernel_id,
                            expr,
                            function.clone(),
                            vec![accumulator, value],
                            globals,
                        )?;
                    }
                    Ok(accumulator)
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "reduce received values outside the supported runtime carriers",
                }),
            },
            BuiltinFoldableCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(initial),
                RuntimeValue::OptionSome(value) => {
                    self.apply_callable(kernel_id, expr, function, vec![initial, *value], globals)
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "reduce received values outside the supported runtime carriers",
                }),
            },
            BuiltinFoldableCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(_) => Ok(initial),
                RuntimeValue::ResultOk(value) => {
                    self.apply_callable(kernel_id, expr, function, vec![initial, *value], globals)
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "reduce received values outside the supported runtime carriers",
                }),
            },
            BuiltinFoldableCarrier::Validation => match strip_signal(subject) {
                RuntimeValue::ValidationInvalid(_) => Ok(initial),
                RuntimeValue::ValidationValid(value) => {
                    self.apply_callable(kernel_id, expr, function, vec![initial, *value], globals)
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "reduce received values outside the supported runtime carriers",
                }),
            },
        }
    }

    fn traverse_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        traversable: BuiltinTraversableCarrier,
        applicative: BuiltinApplicativeCarrier,
        function: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match traversable {
            BuiltinTraversableCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut mapped = Vec::with_capacity(values.len());
                    for value in values {
                        mapped.push(self.apply_callable(
                            kernel_id,
                            expr,
                            function.clone(),
                            vec![value],
                            globals,
                        )?);
                    }
                    sequence_traverse_results(applicative, mapped).map_err(|reason| {
                        EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason,
                        }
                    })
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "traverse received values outside the supported runtime carriers",
                }),
            },
            BuiltinTraversableCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(pure_applicative_value(
                    applicative,
                    RuntimeValue::OptionNone,
                )),
                RuntimeValue::OptionSome(value) => {
                    let mapped =
                        self.apply_callable(kernel_id, expr, function, vec![*value], globals)?;
                    wrap_option_in_applicative(applicative, mapped).map_err(|reason| {
                        EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason,
                        }
                    })
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "traverse received values outside the supported runtime carriers",
                }),
            },
            BuiltinTraversableCarrier::Result => match strip_signal(subject) {
                RuntimeValue::ResultErr(error) => Ok(pure_applicative_value(
                    applicative,
                    RuntimeValue::ResultErr(error),
                )),
                RuntimeValue::ResultOk(value) => {
                    let mapped =
                        self.apply_callable(kernel_id, expr, function, vec![*value], globals)?;
                    wrap_result_ok_in_applicative(applicative, mapped).map_err(|reason| {
                        EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason,
                        }
                    })
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "traverse received values outside the supported runtime carriers",
                }),
            },
            BuiltinTraversableCarrier::Validation => match strip_signal(subject) {
                RuntimeValue::ValidationInvalid(error) => Ok(pure_applicative_value(
                    applicative,
                    RuntimeValue::ValidationInvalid(error),
                )),
                RuntimeValue::ValidationValid(value) => {
                    let mapped =
                        self.apply_callable(kernel_id, expr, function, vec![*value], globals)?;
                    wrap_validation_valid_in_applicative(applicative, mapped).map_err(|reason| {
                        EvaluationError::UnsupportedBuiltinClassMember {
                            kernel: kernel_id,
                            expr,
                            intrinsic,
                            reason,
                        }
                    })
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "traverse received values outside the supported runtime carriers",
                }),
            },
        }
    }

    fn filter_map_builtin_carrier(
        &mut self,
        kernel_id: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        carrier: BuiltinFilterableCarrier,
        function: RuntimeValue,
        subject: RuntimeValue,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        match carrier {
            BuiltinFilterableCarrier::List => match strip_signal(subject) {
                RuntimeValue::List(values) => {
                    let mut filtered = Vec::new();
                    for value in values {
                        match strip_signal(self.apply_callable(
                            kernel_id,
                            expr,
                            function.clone(),
                            vec![value],
                            globals,
                        )?) {
                            RuntimeValue::OptionNone => {}
                            RuntimeValue::OptionSome(value) => filtered.push(*value),
                            _ => {
                                return Err(EvaluationError::UnsupportedBuiltinClassMember {
                                    kernel: kernel_id,
                                    expr,
                                    intrinsic,
                                    reason: "filterMap transforms must evaluate to Option values",
                                });
                            }
                        }
                    }
                    Ok(RuntimeValue::List(filtered))
                }
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "filterMap received values outside the supported runtime carriers",
                }),
            },
            BuiltinFilterableCarrier::Option => match strip_signal(subject) {
                RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                RuntimeValue::OptionSome(value) => match strip_signal(self.apply_callable(
                    kernel_id,
                    expr,
                    function,
                    vec![*value],
                    globals,
                )?) {
                    RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
                    RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(value)),
                    _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                        kernel: kernel_id,
                        expr,
                        intrinsic,
                        reason: "filterMap transforms must evaluate to Option values",
                    }),
                },
                _ => Err(EvaluationError::UnsupportedBuiltinClassMember {
                    kernel: kernel_id,
                    expr,
                    intrinsic,
                    reason: "filterMap received values outside the supported runtime carriers",
                }),
            },
        }
    }

    fn apply_unary(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        operator: UnaryOperator,
        operand: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        let operand = strip_signal(operand);
        match (operator, operand) {
            (UnaryOperator::Not, RuntimeValue::Bool(value)) => Ok(RuntimeValue::Bool(!value)),
            (operator, operand) => Err(EvaluationError::UnsupportedUnary {
                kernel: kernel_id,
                expr,
                operator,
                operand,
            }),
        }
    }

    fn apply_binary(
        &self,
        kernel_id: KernelId,
        expr: KernelExprId,
        operator: BinaryOperator,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, EvaluationError> {
        let left = strip_signal(left);
        let right = strip_signal(right);
        match operator {
            BinaryOperator::Add => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => left
                    .checked_add(*right)
                    .map(RuntimeValue::Int)
                    .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                        kernel: kernel_id,
                        expr,
                        operator,
                        left: RuntimeValue::Int(*left),
                        right: RuntimeValue::Int(*right),
                        reason: "signed addition overflow",
                    }),
                (RuntimeValue::Float(lv), RuntimeValue::Float(rv)) => {
                    RuntimeFloat::new(lv.to_f64() + rv.to_f64())
                        .map(RuntimeValue::Float)
                        .ok_or(EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: RuntimeValue::Float(*lv),
                            right: RuntimeValue::Float(*rv),
                            reason: "float addition result is not finite",
                        })
                }
                _ => apply_i64_like_binary(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left.checked_add(right),
                    "signed addition overflow",
                ),
            },
            BinaryOperator::Subtract => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => left
                    .checked_sub(*right)
                    .map(RuntimeValue::Int)
                    .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                        kernel: kernel_id,
                        expr,
                        operator,
                        left: RuntimeValue::Int(*left),
                        right: RuntimeValue::Int(*right),
                        reason: "signed subtraction overflow",
                    }),
                (RuntimeValue::Float(lv), RuntimeValue::Float(rv)) => {
                    RuntimeFloat::new(lv.to_f64() - rv.to_f64())
                        .map(RuntimeValue::Float)
                        .ok_or(EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: RuntimeValue::Float(*lv),
                            right: RuntimeValue::Float(*rv),
                            reason: "float subtraction result is not finite",
                        })
                }
                _ => apply_i64_like_binary(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left.checked_sub(right),
                    "signed subtraction overflow",
                ),
            },
            BinaryOperator::Multiply => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => left
                    .checked_mul(*right)
                    .map(RuntimeValue::Int)
                    .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                        kernel: kernel_id,
                        expr,
                        operator,
                        left: RuntimeValue::Int(*left),
                        right: RuntimeValue::Int(*right),
                        reason: "signed multiplication overflow",
                    }),
                (RuntimeValue::Float(lv), RuntimeValue::Float(rv)) => {
                    RuntimeFloat::new(lv.to_f64() * rv.to_f64())
                        .map(RuntimeValue::Float)
                        .ok_or(EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: RuntimeValue::Float(*lv),
                            right: RuntimeValue::Float(*rv),
                            reason: "float multiplication result is not finite",
                        })
                }
                _ => apply_i64_like_binary(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left.checked_mul(right),
                    "signed multiplication overflow",
                ),
            },
            BinaryOperator::Divide => match (&left, &right) {
                (RuntimeValue::Int(left_int), RuntimeValue::Int(right_int)) => left_int
                    .checked_div(*right_int)
                    .map(RuntimeValue::Int)
                    .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                        kernel: kernel_id,
                        expr,
                        operator,
                        left: left.clone(),
                        right: right.clone(),
                        reason: if *right_int == 0 {
                            "division by zero"
                        } else {
                            "signed division overflow"
                        },
                    }),
                (RuntimeValue::Float(lf), RuntimeValue::Float(rf)) => {
                    RuntimeFloat::new(lf.to_f64() / rf.to_f64())
                        .map(RuntimeValue::Float)
                        .ok_or(EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: RuntimeValue::Float(*lf),
                            right: RuntimeValue::Float(*rf),
                            reason: "float division result is not finite",
                        })
                }
                _ => {
                    let Some((left_int, right_int, preserved_suffix)) =
                        coerce_i64_like_operands(&left, &right)
                    else {
                        return Err(EvaluationError::UnsupportedBinary {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left,
                            right,
                        });
                    };
                    left_int
                        .checked_div(right_int)
                        .map(|value| runtime_i64_like_value(value, preserved_suffix))
                        .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: left.clone(),
                            right: right.clone(),
                            reason: if right_int == 0 {
                                "division by zero"
                            } else {
                                "signed division overflow"
                            },
                        })
                }
            },
            BinaryOperator::Modulo => match (&left, &right) {
                (RuntimeValue::Int(left_int), RuntimeValue::Int(right_int)) => left_int
                    .checked_rem(*right_int)
                    .map(RuntimeValue::Int)
                    .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                        kernel: kernel_id,
                        expr,
                        operator,
                        left: left.clone(),
                        right: right.clone(),
                        reason: if *right_int == 0 {
                            "modulo by zero"
                        } else {
                            "signed remainder overflow"
                        },
                    }),
                _ => {
                    let Some((left_int, right_int, preserved_suffix)) =
                        coerce_i64_like_operands(&left, &right)
                    else {
                        return Err(EvaluationError::UnsupportedBinary {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left,
                            right,
                        });
                    };
                    left_int
                        .checked_rem(right_int)
                        .map(|value| runtime_i64_like_value(value, preserved_suffix))
                        .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
                            kernel: kernel_id,
                            expr,
                            operator,
                            left: left.clone(),
                            right: right.clone(),
                            reason: if right_int == 0 {
                                "modulo by zero"
                            } else {
                                "signed remainder overflow"
                            },
                        })
                }
            },
            BinaryOperator::GreaterThan => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left > right))
                }
                (RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                    Ok(RuntimeValue::Bool(left.to_f64() > right.to_f64()))
                }
                _ => apply_i64_like_comparison(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left > right,
                ),
            },
            BinaryOperator::LessThan => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left < right))
                }
                (RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                    Ok(RuntimeValue::Bool(left.to_f64() < right.to_f64()))
                }
                _ => apply_i64_like_comparison(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left < right,
                ),
            },
            BinaryOperator::GreaterThanOrEqual => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left >= right))
                }
                (RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                    Ok(RuntimeValue::Bool(left.to_f64() >= right.to_f64()))
                }
                _ => apply_i64_like_comparison(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left >= right,
                ),
            },
            BinaryOperator::LessThanOrEqual => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left <= right))
                }
                (RuntimeValue::Float(left), RuntimeValue::Float(right)) => {
                    Ok(RuntimeValue::Bool(left.to_f64() <= right.to_f64()))
                }
                _ => apply_i64_like_comparison(
                    kernel_id,
                    expr,
                    operator,
                    &left,
                    &right,
                    |left, right| left <= right,
                ),
            },
            BinaryOperator::And => match (&left, &right) {
                (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => {
                    Ok(RuntimeValue::Bool(*left && *right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
            },
            BinaryOperator::Or => match (&left, &right) {
                (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => {
                    Ok(RuntimeValue::Bool(*left || *right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
            },
            BinaryOperator::Equals | BinaryOperator::NotEquals => {
                let equal = structural_eq(kernel_id, expr, &left, &right)?;
                Ok(RuntimeValue::Bool(
                    if matches!(operator, BinaryOperator::Equals) {
                        equal
                    } else {
                        !equal
                    },
                ))
            }
        }
    }
}

fn apply_i64_like_binary(
    kernel: KernelId,
    expr: KernelExprId,
    operator: BinaryOperator,
    left: &RuntimeValue,
    right: &RuntimeValue,
    operation: impl FnOnce(i64, i64) -> Option<i64>,
    overflow_reason: &'static str,
) -> Result<RuntimeValue, EvaluationError> {
    let Some((left_int, right_int, preserved_suffix)) = coerce_i64_like_operands(left, right)
    else {
        return Err(EvaluationError::UnsupportedBinary {
            kernel,
            expr,
            operator,
            left: left.clone(),
            right: right.clone(),
        });
    };
    operation(left_int, right_int)
        .map(|value| runtime_i64_like_value(value, preserved_suffix))
        .ok_or_else(|| EvaluationError::InvalidBinaryArithmetic {
            kernel,
            expr,
            operator,
            left: left.clone(),
            right: right.clone(),
            reason: overflow_reason,
        })
}

fn apply_i64_like_comparison(
    kernel: KernelId,
    expr: KernelExprId,
    operator: BinaryOperator,
    left: &RuntimeValue,
    right: &RuntimeValue,
    comparison: impl FnOnce(i64, i64) -> bool,
) -> Result<RuntimeValue, EvaluationError> {
    let Some((left_int, right_int, _)) = coerce_i64_like_operands(left, right) else {
        return Err(EvaluationError::UnsupportedBinary {
            kernel,
            expr,
            operator,
            left: left.clone(),
            right: right.clone(),
        });
    };
    Ok(RuntimeValue::Bool(comparison(left_int, right_int)))
}

fn coerce_i64_like_operands(
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Option<(i64, i64, Option<Box<str>>)> {
    let preserved_suffix = shared_suffixed_integer_suffix(left, right)?;
    let left = coerce_i64_like_value(left.clone())?;
    let right = coerce_i64_like_value(right.clone())?;
    Some((left, right, preserved_suffix))
}

fn coerce_i64_like_value(value: RuntimeValue) -> Option<i64> {
    match strip_signal(value) {
        RuntimeValue::Int(value) => Some(value),
        RuntimeValue::SuffixedInteger { raw, .. } => raw.parse::<i64>().ok(),
        _ => None,
    }
}

fn runtime_i64_like_value(value: i64, preserved_suffix: Option<Box<str>>) -> RuntimeValue {
    match preserved_suffix {
        Some(suffix) => RuntimeValue::SuffixedInteger {
            raw: value.to_string().into_boxed_str(),
            suffix,
        },
        None => RuntimeValue::Int(value),
    }
}

fn map_builtin(term: BuiltinTerm) -> RuntimeValue {
    match term {
        BuiltinTerm::True => RuntimeValue::Bool(true),
        BuiltinTerm::False => RuntimeValue::Bool(false),
        BuiltinTerm::None => RuntimeValue::OptionNone,
        BuiltinTerm::Some => RuntimeValue::Callable(RuntimeCallable::BuiltinConstructor {
            constructor: RuntimeConstructor::Some,
            bound_arguments: Vec::new(),
        }),
        BuiltinTerm::Ok => RuntimeValue::Callable(RuntimeCallable::BuiltinConstructor {
            constructor: RuntimeConstructor::Ok,
            bound_arguments: Vec::new(),
        }),
        BuiltinTerm::Err => RuntimeValue::Callable(RuntimeCallable::BuiltinConstructor {
            constructor: RuntimeConstructor::Err,
            bound_arguments: Vec::new(),
        }),
        BuiltinTerm::Valid => RuntimeValue::Callable(RuntimeCallable::BuiltinConstructor {
            constructor: RuntimeConstructor::Valid,
            bound_arguments: Vec::new(),
        }),
        BuiltinTerm::Invalid => RuntimeValue::Callable(RuntimeCallable::BuiltinConstructor {
            constructor: RuntimeConstructor::Invalid,
            bound_arguments: Vec::new(),
        }),
    }
}

fn runtime_intrinsic_value(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
) -> Result<RuntimeValue, EvaluationError> {
    if intrinsic_value_arity(value) == 0 {
        evaluate_intrinsic_value(kernel, expr, value, Vec::new())
    } else {
        Ok(RuntimeValue::Callable(RuntimeCallable::IntrinsicValue {
            value,
            bound_arguments: Vec::new(),
        }))
    }
}

fn runtime_class_member_value(intrinsic: BuiltinClassMemberIntrinsic) -> RuntimeValue {
    match intrinsic {
        BuiltinClassMemberIntrinsic::Empty(BuiltinAppendCarrier::Text) => {
            RuntimeValue::Text("".into())
        }
        BuiltinClassMemberIntrinsic::Empty(BuiltinAppendCarrier::List) => {
            RuntimeValue::List(Vec::new())
        }
        _ => RuntimeValue::Callable(RuntimeCallable::BuiltinClassMember {
            intrinsic,
            bound_arguments: Vec::new(),
        }),
    }
}

fn intrinsic_value_arity(value: IntrinsicValue) -> usize {
    match value {
        IntrinsicValue::TupleConstructor { arity } => arity,
        IntrinsicValue::CustomCapabilityCommand(spec) => {
            spec.provider_arguments.len() + spec.options.len() + spec.arguments.len()
        }
        IntrinsicValue::RandomBytes => 1,
        IntrinsicValue::RandomInt => 2,
        IntrinsicValue::StdoutWrite => 1,
        IntrinsicValue::StderrWrite => 1,
        IntrinsicValue::FsWriteText => 2,
        IntrinsicValue::FsWriteBytes => 2,
        IntrinsicValue::FsCreateDirAll => 1,
        IntrinsicValue::FsDeleteFile => 1,
        IntrinsicValue::DbParamBool
        | IntrinsicValue::DbParamInt
        | IntrinsicValue::DbParamFloat
        | IntrinsicValue::DbParamDecimal
        | IntrinsicValue::DbParamBigInt
        | IntrinsicValue::DbParamText
        | IntrinsicValue::DbParamBytes => 1,
        IntrinsicValue::DbStatement => 2,
        IntrinsicValue::DbQuery => 2,
        IntrinsicValue::DbCommit => 3,
        IntrinsicValue::FloatFloor
        | IntrinsicValue::FloatCeil
        | IntrinsicValue::FloatRound
        | IntrinsicValue::FloatSqrt
        | IntrinsicValue::FloatAbs
        | IntrinsicValue::FloatToInt
        | IntrinsicValue::FloatFromInt
        | IntrinsicValue::FloatToText
        | IntrinsicValue::FloatParseText => 1,
        IntrinsicValue::FsReadText => 1,
        IntrinsicValue::FsReadDir => 1,
        IntrinsicValue::FsExists => 1,
        IntrinsicValue::FsReadBytes => 1,
        IntrinsicValue::FsRename => 2,
        IntrinsicValue::FsCopy => 2,
        IntrinsicValue::FsDeleteDir => 1,
        IntrinsicValue::PathParent => 1,
        IntrinsicValue::PathFilename => 1,
        IntrinsicValue::PathStem => 1,
        IntrinsicValue::PathExtension => 1,
        IntrinsicValue::PathJoin => 2,
        IntrinsicValue::PathIsAbsolute => 1,
        IntrinsicValue::PathNormalize => 1,
        IntrinsicValue::BytesLength => 1,
        IntrinsicValue::BytesGet => 2,
        IntrinsicValue::BytesSlice => 3,
        IntrinsicValue::BytesAppend => 2,
        IntrinsicValue::BytesFromText => 1,
        IntrinsicValue::BytesToText => 1,
        IntrinsicValue::BytesRepeat => 2,
        IntrinsicValue::BytesEmpty => 0,
        IntrinsicValue::JsonValidate => 1,
        IntrinsicValue::JsonGet => 2,
        IntrinsicValue::JsonAt => 2,
        IntrinsicValue::JsonKeys => 1,
        IntrinsicValue::JsonPretty => 1,
        IntrinsicValue::JsonMinify => 1,
        IntrinsicValue::XdgDataHome => 0,
        IntrinsicValue::XdgConfigHome => 0,
        IntrinsicValue::XdgCacheHome => 0,
        IntrinsicValue::XdgStateHome => 0,
        IntrinsicValue::XdgRuntimeDir => 0,
        IntrinsicValue::XdgDataDirs => 0,
        IntrinsicValue::XdgConfigDirs => 0,
        // Text intrinsics
        IntrinsicValue::TextLength
        | IntrinsicValue::TextByteLen
        | IntrinsicValue::TextToUpper
        | IntrinsicValue::TextToLower
        | IntrinsicValue::TextTrim
        | IntrinsicValue::TextTrimStart
        | IntrinsicValue::TextTrimEnd
        | IntrinsicValue::TextFromInt
        | IntrinsicValue::TextParseInt
        | IntrinsicValue::TextFromBool
        | IntrinsicValue::TextParseBool
        | IntrinsicValue::TextConcat
        | IntrinsicValue::I18nTranslate => 1,
        IntrinsicValue::TextFind
        | IntrinsicValue::TextContains
        | IntrinsicValue::TextStartsWith
        | IntrinsicValue::TextEndsWith
        | IntrinsicValue::TextSplit
        | IntrinsicValue::TextRepeat
        | IntrinsicValue::I18nTranslatePlural => 2,
        IntrinsicValue::TextSlice
        | IntrinsicValue::TextReplace
        | IntrinsicValue::TextReplaceAll => 3,
        // Float transcendental intrinsics
        IntrinsicValue::FloatSin
        | IntrinsicValue::FloatCos
        | IntrinsicValue::FloatTan
        | IntrinsicValue::FloatAsin
        | IntrinsicValue::FloatAcos
        | IntrinsicValue::FloatAtan
        | IntrinsicValue::FloatExp
        | IntrinsicValue::FloatLog
        | IntrinsicValue::FloatLog2
        | IntrinsicValue::FloatLog10
        | IntrinsicValue::FloatTrunc
        | IntrinsicValue::FloatFrac => 1,
        IntrinsicValue::FloatAtan2 | IntrinsicValue::FloatPow | IntrinsicValue::FloatHypot => 2,
        // Time intrinsics
        IntrinsicValue::TimeNowMs
        | IntrinsicValue::TimeMonotonicMs
        | IntrinsicValue::RandomFloat => 0,
        IntrinsicValue::TimeFormat | IntrinsicValue::TimeParse => 2,
        // Env intrinsics
        IntrinsicValue::EnvGet | IntrinsicValue::EnvList => 1,
        // Log intrinsics
        IntrinsicValue::LogEmit => 2,
        IntrinsicValue::LogEmitContext => 3,
        // Regex intrinsics
        IntrinsicValue::RegexIsMatch
        | IntrinsicValue::RegexFind
        | IntrinsicValue::RegexFindText
        | IntrinsicValue::RegexFindAll => 2,
        IntrinsicValue::RegexReplace | IntrinsicValue::RegexReplaceAll => 3,
        // HTTP intrinsics
        IntrinsicValue::HttpGet
        | IntrinsicValue::HttpGetBytes
        | IntrinsicValue::HttpGetStatus
        | IntrinsicValue::HttpDelete
        | IntrinsicValue::HttpHead => 1,
        IntrinsicValue::HttpPostJson => 2,
        IntrinsicValue::HttpPost | IntrinsicValue::HttpPut => 3,
        // BigInt intrinsics
        IntrinsicValue::BigIntFromInt
        | IntrinsicValue::BigIntFromText
        | IntrinsicValue::BigIntToInt
        | IntrinsicValue::BigIntToText
        | IntrinsicValue::BigIntNeg
        | IntrinsicValue::BigIntAbs => 1,
        IntrinsicValue::BigIntAdd
        | IntrinsicValue::BigIntSub
        | IntrinsicValue::BigIntMul
        | IntrinsicValue::BigIntDiv
        | IntrinsicValue::BigIntMod
        | IntrinsicValue::BigIntPow
        | IntrinsicValue::BigIntCmp
        | IntrinsicValue::BigIntEq
        | IntrinsicValue::BigIntGt
        | IntrinsicValue::BigIntLt => 2,
    }
}
