use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use aivi_hir::DomainMemberHandle;

use crate::{
    BinaryOperator, BuiltinTerm, EnvSlotId, InlinePipePattern, InlinePipePatternKind,
    InlinePipeStageKind, InlineSubjectId, ItemId, KernelExprId, KernelExprKind, KernelId, LayoutId,
    LayoutKind, PrimitiveType, Program, ProjectionBase, SubjectRef, UnaryOperator,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRecordField {
    pub label: Box<str>,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeMapEntry {
    pub key: RuntimeValue,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeConstructor {
    Some,
    Ok,
    Err,
    Valid,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeCallable {
    ItemBody {
        item: ItemId,
        kernel: KernelId,
        parameters: Vec<LayoutId>,
        bound_arguments: Vec<RuntimeValue>,
    },
    BuiltinConstructor {
        constructor: RuntimeConstructor,
        bound_arguments: Vec<RuntimeValue>,
    },
    DomainMember {
        handle: DomainMemberHandle,
        bound_arguments: Vec<RuntimeValue>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeValue {
    Unit,
    Bool(bool),
    Int(i64),
    Text(Box<str>),
    Tuple(Vec<RuntimeValue>),
    List(Vec<RuntimeValue>),
    Map(Vec<RuntimeMapEntry>),
    Set(Vec<RuntimeValue>),
    Record(Vec<RuntimeRecordField>),
    OptionNone,
    OptionSome(Box<RuntimeValue>),
    ResultOk(Box<RuntimeValue>),
    ResultErr(Box<RuntimeValue>),
    ValidationValid(Box<RuntimeValue>),
    ValidationInvalid(Box<RuntimeValue>),
    Signal(Box<RuntimeValue>),
    SuffixedInteger { raw: Box<str>, suffix: Box<str> },
    Callable(RuntimeCallable),
}

impl RuntimeValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_ref()),
            _ => None,
        }
    }

    fn write_display_text(&self, target: &mut impl fmt::Write) -> fmt::Result {
        let mut stack = vec![DisplayFrame::Value(self)];
        while let Some(frame) = stack.pop() {
            match frame {
                DisplayFrame::Value(value) => match value {
                    Self::Unit => target.write_str("()")?,
                    Self::Bool(true) => target.write_str("True")?,
                    Self::Bool(false) => target.write_str("False")?,
                    Self::Int(value) => write!(target, "{value}")?,
                    Self::Text(value) => target.write_str(value)?,
                    Self::Tuple(elements) => {
                        push_delimited_values(&mut stack, elements, "(", ")");
                    }
                    Self::List(elements) => {
                        push_delimited_values(&mut stack, elements, "[", "]");
                    }
                    Self::Map(entries) => {
                        push_map_entries(&mut stack, entries);
                    }
                    Self::Set(elements) => {
                        push_delimited_values(&mut stack, elements, "#", "");
                    }
                    Self::Record(fields) => {
                        push_record_fields(&mut stack, fields);
                    }
                    Self::OptionNone => target.write_str("None")?,
                    Self::OptionSome(value) => {
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Some "));
                    }
                    Self::ResultOk(value) => {
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Ok "));
                    }
                    Self::ResultErr(value) => {
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Err "));
                    }
                    Self::ValidationValid(value) => {
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Valid "));
                    }
                    Self::ValidationInvalid(value) => {
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Invalid "));
                    }
                    Self::Signal(value) => {
                        stack.push(DisplayFrame::StaticText(")"));
                        stack.push(DisplayFrame::Value(value));
                        stack.push(DisplayFrame::StaticText("Signal("));
                    }
                    Self::SuffixedInteger { raw, suffix } => write!(target, "{raw}{suffix}")?,
                    Self::Callable(callable) => match callable {
                        RuntimeCallable::ItemBody { item, .. } => {
                            write!(target, "<item-body item{item}>")?
                        }
                        RuntimeCallable::BuiltinConstructor { constructor, .. } => {
                            write!(target, "<constructor {constructor}>")?
                        }
                        RuntimeCallable::DomainMember { handle, .. } => write!(
                            target,
                            "<domain-member {}.{}>",
                            handle.domain_name, handle.member_name
                        )?,
                    },
                },
                DisplayFrame::StaticText(text) => target.write_str(text)?,
                DisplayFrame::BorrowedText(text) => target.write_str(text)?,
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn display_text(&self) -> String {
        let mut rendered = String::new();
        self.write_display_text(&mut rendered)
            .expect("writing into a String should not fail");
        rendered
    }
}

impl fmt::Display for RuntimeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_display_text(f)
    }
}

enum DisplayFrame<'a> {
    Value(&'a RuntimeValue),
    StaticText(&'static str),
    BorrowedText(&'a str),
}

fn push_delimited_values<'a>(
    stack: &mut Vec<DisplayFrame<'a>>,
    values: &'a [RuntimeValue],
    open: &'static str,
    close: &'static str,
) {
    stack.push(DisplayFrame::StaticText(close));
    for (index, value) in values.iter().enumerate().rev() {
        stack.push(DisplayFrame::Value(value));
        if index > 0 {
            stack.push(DisplayFrame::StaticText(", "));
        }
    }
    stack.push(DisplayFrame::StaticText(open));
}

fn push_map_entries<'a>(stack: &mut Vec<DisplayFrame<'a>>, entries: &'a [RuntimeMapEntry]) {
    stack.push(DisplayFrame::StaticText("}"));
    for (index, entry) in entries.iter().enumerate().rev() {
        stack.push(DisplayFrame::Value(&entry.value));
        stack.push(DisplayFrame::StaticText(": "));
        stack.push(DisplayFrame::Value(&entry.key));
        if index > 0 {
            stack.push(DisplayFrame::StaticText(", "));
        }
    }
    stack.push(DisplayFrame::StaticText("{"));
}

fn push_record_fields<'a>(stack: &mut Vec<DisplayFrame<'a>>, fields: &'a [RuntimeRecordField]) {
    stack.push(DisplayFrame::StaticText("}"));
    for (index, field) in fields.iter().enumerate().rev() {
        stack.push(DisplayFrame::Value(&field.value));
        stack.push(DisplayFrame::StaticText(": "));
        stack.push(DisplayFrame::BorrowedText(field.label.as_ref()));
        if index > 0 {
            stack.push(DisplayFrame::StaticText(", "));
        }
    }
    stack.push(DisplayFrame::StaticText("{"));
}

impl fmt::Display for RuntimeConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Some => f.write_str("Some"),
            Self::Ok => f.write_str("Ok"),
            Self::Err => f.write_str("Err"),
            Self::Valid => f.write_str("Valid"),
            Self::Invalid => f.write_str("Invalid"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationError {
    UnknownKernel {
        kernel: KernelId,
    },
    UnknownItem {
        item: ItemId,
    },
    MissingItemBody {
        item: ItemId,
    },
    MissingItemValue {
        item: ItemId,
    },
    RecursiveItemEvaluation {
        item: ItemId,
    },
    MissingInputSubject {
        kernel: KernelId,
    },
    UnexpectedInputSubject {
        kernel: KernelId,
    },
    KernelEnvironmentCountMismatch {
        kernel: KernelId,
        expected: usize,
        found: usize,
    },
    KernelInputLayoutMismatch {
        kernel: KernelId,
        expected: LayoutId,
        found: RuntimeValue,
    },
    KernelEnvironmentLayoutMismatch {
        kernel: KernelId,
        slot: EnvSlotId,
        expected: LayoutId,
        found: RuntimeValue,
    },
    KernelResultLayoutMismatch {
        kernel: KernelId,
        expected: LayoutId,
        found: RuntimeValue,
    },
    UnknownEnvironmentSlot {
        kernel: KernelId,
        expr: KernelExprId,
        slot: EnvSlotId,
    },
    UnknownInlineSubject {
        kernel: KernelId,
        expr: KernelExprId,
        slot: InlineSubjectId,
    },
    UnknownProjectionField {
        kernel: KernelId,
        expr: KernelExprId,
        label: Box<str>,
    },
    InvalidProjectionBase {
        kernel: KernelId,
        expr: KernelExprId,
        found: RuntimeValue,
    },
    InvalidCallee {
        kernel: KernelId,
        expr: KernelExprId,
        found: RuntimeValue,
    },
    UnsupportedDomainMemberCall {
        kernel: KernelId,
        expr: KernelExprId,
        handle: DomainMemberHandle,
    },
    UnsupportedInlinePipe {
        kernel: KernelId,
        expr: KernelExprId,
    },
    UnsupportedInlinePipeSignalSubject {
        kernel: KernelId,
        expr: KernelExprId,
        found: RuntimeValue,
    },
    UnsupportedInlinePipePattern {
        kernel: KernelId,
        expr: KernelExprId,
    },
    InlinePipeCaseNoMatch {
        kernel: KernelId,
        expr: KernelExprId,
        subject: RuntimeValue,
    },
    UnsupportedUnary {
        kernel: KernelId,
        expr: KernelExprId,
        operator: UnaryOperator,
        operand: RuntimeValue,
    },
    UnsupportedBinary {
        kernel: KernelId,
        expr: KernelExprId,
        operator: BinaryOperator,
        left: RuntimeValue,
        right: RuntimeValue,
    },
    InvalidInterpolationValue {
        kernel: KernelId,
        expr: KernelExprId,
        found: RuntimeValue,
    },
    InvalidIntegerLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    UnsupportedStructuralEquality {
        kernel: KernelId,
        expr: KernelExprId,
        left: RuntimeValue,
        right: RuntimeValue,
    },
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKernel { kernel } => write!(f, "unknown backend kernel {kernel}"),
            Self::UnknownItem { item } => write!(f, "unknown backend item {item}"),
            Self::MissingItemBody { item } => {
                write!(f, "backend item {item} has no lowered body kernel")
            }
            Self::MissingItemValue { item } => write!(
                f,
                "backend item {item} needs a runtime value, but no override or lowered body exists"
            ),
            Self::RecursiveItemEvaluation { item } => {
                write!(
                    f,
                    "backend item {item} recursively depends on itself at runtime"
                )
            }
            Self::MissingInputSubject { kernel } => {
                write!(f, "kernel {kernel} requires an input subject")
            }
            Self::UnexpectedInputSubject { kernel } => {
                write!(f, "kernel {kernel} does not accept an input subject")
            }
            Self::KernelEnvironmentCountMismatch {
                kernel,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} expected {expected} environment slot(s), found {found}"
            ),
            Self::KernelInputLayoutMismatch {
                kernel,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} expected input layout {expected}, found runtime value `{found}`"
            ),
            Self::KernelEnvironmentLayoutMismatch {
                kernel,
                slot,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} expected environment slot {slot} to match layout {expected}, found `{found}`"
            ),
            Self::KernelResultLayoutMismatch {
                kernel,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} produced runtime value `{found}` that does not match layout {expected}"
            ),
            Self::UnknownEnvironmentSlot { kernel, slot, .. } => {
                write!(
                    f,
                    "kernel {kernel} references missing environment slot {slot}"
                )
            }
            Self::UnknownInlineSubject { kernel, slot, .. } => {
                write!(
                    f,
                    "kernel {kernel} references missing inline subject {slot}"
                )
            }
            Self::UnknownProjectionField { kernel, label, .. } => {
                write!(
                    f,
                    "kernel {kernel} projected missing record field `{label}`"
                )
            }
            Self::InvalidProjectionBase { kernel, found, .. } => write!(
                f,
                "kernel {kernel} can only project records in the current runtime slice, found `{found}`"
            ),
            Self::InvalidCallee { kernel, found, .. } => write!(
                f,
                "kernel {kernel} attempted to call non-callable runtime value `{found}`"
            ),
            Self::UnsupportedDomainMemberCall { kernel, handle, .. } => write!(
                f,
                "kernel {kernel} reached unresolved domain member {}.{} at runtime",
                handle.domain_name, handle.member_name
            ),
            Self::UnsupportedInlinePipe { kernel, .. } => write!(
                f,
                "kernel {kernel} still contains an inline pipe; runtime evaluation for inline pipes remains a later backend slice"
            ),
            Self::UnsupportedInlinePipeSignalSubject { kernel, found, .. } => write!(
                f,
                "kernel {kernel} reached an inline pipe over signal subject `{found}`, but snapshot-time signal inline pipes remain a later runtime slice"
            ),
            Self::UnsupportedInlinePipePattern { kernel, .. } => write!(
                f,
                "kernel {kernel} reached an inline case pattern that the current runtime evaluator cannot match"
            ),
            Self::InlinePipeCaseNoMatch {
                kernel, subject, ..
            } => write!(
                f,
                "kernel {kernel} evaluated an inline case with no matching arm for `{subject}`"
            ),
            Self::UnsupportedUnary {
                kernel,
                operator,
                operand,
                ..
            } => write!(
                f,
                "kernel {kernel} cannot apply unary operator `{operator}` to `{operand}`"
            ),
            Self::UnsupportedBinary {
                kernel,
                operator,
                left,
                right,
                ..
            } => write!(
                f,
                "kernel {kernel} cannot apply binary operator `{operator}` to `{left}` and `{right}`"
            ),
            Self::InvalidInterpolationValue { kernel, found, .. } => write!(
                f,
                "kernel {kernel} cannot interpolate callable runtime value `{found}` into text"
            ),
            Self::InvalidIntegerLiteral { kernel, raw, .. } => {
                write!(f, "kernel {kernel} could not parse integer literal `{raw}`")
            }
            Self::UnsupportedStructuralEquality {
                kernel,
                left,
                right,
                ..
            } => write!(
                f,
                "kernel {kernel} cannot compare `{left}` and `{right}` structurally in the current runtime slice"
            ),
        }
    }
}

impl std::error::Error for EvaluationError {}

pub struct KernelEvaluator<'a> {
    program: &'a Program,
    item_cache: BTreeMap<ItemId, RuntimeValue>,
    item_stack: BTreeSet<ItemId>,
}

impl<'a> KernelEvaluator<'a> {
    pub fn new(program: &'a Program) -> Self {
        Self {
            program,
            item_cache: BTreeMap::new(),
            item_stack: BTreeSet::new(),
        }
    }

    pub fn program(&self) -> &'a Program {
        self.program
    }

    pub fn evaluate_kernel(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
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
        let inline_subjects = vec![None; kernel.inline_subjects.len()];
        let result = self.evaluate_expr(
            kernel_id,
            kernel.root,
            input_subject,
            environment,
            &inline_subjects,
            globals,
        )?;
        if !value_matches_layout(self.program, &result, kernel.result_layout) {
            return Err(EvaluationError::KernelResultLayoutMismatch {
                kernel: kernel_id,
                expected: kernel.result_layout,
                found: result,
            });
        }
        Ok(result)
    }

    pub fn evaluate_item(
        &mut self,
        item: ItemId,
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        if let Some(value) = globals.get(&item) {
            return Ok(value.clone());
        }
        if let Some(value) = self.item_cache.get(&item) {
            return Ok(value.clone());
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
            return Err(EvaluationError::RecursiveItemEvaluation { item });
        }
        let result = self.evaluate_kernel(kernel, None, &[], globals);
        self.item_stack.remove(&item);
        let result = result?;
        self.item_cache.insert(item, result.clone());
        Ok(result)
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
                        KernelExprKind::DomainMember(handle) => {
                            values.push(RuntimeValue::Callable(RuntimeCallable::DomainMember {
                                handle: handle.clone(),
                                bound_arguments: Vec::new(),
                            }))
                        }
                        KernelExprKind::Builtin(term) => values.push(map_builtin(*term)),
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
                            let base_build = match base {
                                ProjectionBase::Subject(subject) => {
                                    ProjectionBuild::Subject(*subject)
                                }
                                ProjectionBase::Expr(inner) => {
                                    tasks.push(Task::Visit(*inner));
                                    ProjectionBuild::Expr
                                }
                            };
                            tasks.push(Task::BuildProjection {
                                expr: expr_id,
                                base: base_build,
                                path: path.clone(),
                            });
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
                    values.push(RuntimeValue::Map(entries));
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
        for stage in &pipe.stages {
            if matches!(current, RuntimeValue::Signal(_)) {
                return Err(EvaluationError::UnsupportedInlinePipeSignalSubject {
                    kernel: kernel_id,
                    expr: expr_id,
                    found: current,
                });
            }
            let mut stage_subjects = inline_subjects.to_vec();
            stage_subjects[stage.subject.index()] = Some(current.clone());
            current = match &stage.kind {
                InlinePipeStageKind::Transform { expr } => self.evaluate_expr(
                    kernel_id,
                    *expr,
                    input_subject,
                    environment,
                    &stage_subjects,
                    globals,
                )?,
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
            };
            if !value_matches_layout(self.program, &current, stage.result_layout) {
                return Err(EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: stage.result_layout,
                    found: current,
                });
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
            } => {
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
        let arguments = arguments.into_iter().map(strip_signal).collect::<Vec<_>>();
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
                bound_arguments.extend(arguments);
                if bound_arguments.len() < parameters.len() {
                    return Ok(RuntimeValue::Callable(RuntimeCallable::ItemBody {
                        item,
                        kernel,
                        parameters,
                        bound_arguments,
                    }));
                }
                let remaining = bound_arguments.split_off(parameters.len());
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
                bound_arguments.extend(arguments);
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
            RuntimeCallable::DomainMember {
                handle,
                bound_arguments,
            } => {
                let _ = bound_arguments;
                Err(EvaluationError::UnsupportedDomainMemberCall {
                    kernel: kernel_id,
                    expr,
                    handle,
                })
            }
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
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Int(left + right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
            },
            BinaryOperator::Subtract => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Int(left - right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
            },
            BinaryOperator::GreaterThan => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left > right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
            },
            BinaryOperator::LessThan => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Bool(left < right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
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

fn value_matches_layout(program: &Program, value: &RuntimeValue, layout: LayoutId) -> bool {
    let Some(layout) = program.layouts().get(layout) else {
        return false;
    };
    match (&layout.kind, value) {
        (LayoutKind::Primitive(PrimitiveType::Unit), RuntimeValue::Unit) => true,
        (LayoutKind::Primitive(PrimitiveType::Bool), RuntimeValue::Bool(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Int), RuntimeValue::Int(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Text), RuntimeValue::Text(_)) => true,
        (LayoutKind::Tuple(expected), RuntimeValue::Tuple(elements)) => {
            expected.len() == elements.len()
                && expected
                    .iter()
                    .zip(elements.iter())
                    .all(|(layout, value)| value_matches_layout(program, value, *layout))
        }
        (LayoutKind::List { element }, RuntimeValue::List(elements))
        | (LayoutKind::Set { element }, RuntimeValue::Set(elements)) => elements
            .iter()
            .all(|value| value_matches_layout(program, value, *element)),
        (LayoutKind::Map { key, value }, RuntimeValue::Map(entries)) => {
            entries.iter().all(|entry| {
                value_matches_layout(program, &entry.key, *key)
                    && value_matches_layout(program, &entry.value, *value)
            })
        }
        (LayoutKind::Record(expected), RuntimeValue::Record(fields)) => {
            expected.len() == fields.len()
                && expected.iter().zip(fields.iter()).all(|(layout, field)| {
                    layout.name.as_ref() == field.label.as_ref()
                        && value_matches_layout(program, &field.value, layout.layout)
                })
        }
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
        (LayoutKind::Arrow { .. }, RuntimeValue::Callable(_)) => true,
        (LayoutKind::AnonymousDomain { .. }, RuntimeValue::SuffixedInteger { .. })
        | (LayoutKind::Domain { .. }, RuntimeValue::SuffixedInteger { .. }) => true,
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
        (RuntimeValue::Text(left), RuntimeValue::Text(right)) => left == right,
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
        | (RuntimeValue::List(left), RuntimeValue::List(right))
        | (RuntimeValue::Set(left), RuntimeValue::Set(right)) => {
            if left.len() != right.len() {
                false
            } else {
                let mut equal = true;
                for (left, right) in left.iter().zip(right.iter()) {
                    equal &= structural_eq(kernel, expr, left, right)?;
                }
                equal
            }
        }
        (RuntimeValue::Record(left), RuntimeValue::Record(right)) => {
            if left.len() != right.len() {
                false
            } else {
                let mut equal = true;
                for (left, right) in left.iter().zip(right.iter()) {
                    equal &= left.label == right.label;
                    equal &= structural_eq(kernel, expr, &left.value, &right.value)?;
                }
                equal
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
        _ => {
            return Err(EvaluationError::UnsupportedStructuralEquality {
                kernel,
                expr,
                left: left.clone(),
                right: right.clone(),
            });
        }
    };
    Ok(equal)
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

fn strip_signal(value: RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(value) => *value,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeMapEntry, RuntimeRecordField, RuntimeValue};

    #[test]
    fn display_formats_nested_runtime_values_without_intermediate_joining() {
        let value = RuntimeValue::Record(vec![
            RuntimeRecordField {
                label: "status".into(),
                value: RuntimeValue::OptionSome(Box::new(RuntimeValue::ResultOk(Box::new(
                    RuntimeValue::Tuple(vec![
                        RuntimeValue::Int(1),
                        RuntimeValue::Text("ok".into()),
                    ]),
                )))),
            },
            RuntimeRecordField {
                label: "metadata".into(),
                value: RuntimeValue::Map(vec![RuntimeMapEntry {
                    key: RuntimeValue::Text("attempts".into()),
                    value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
                }]),
            },
        ]);

        assert_eq!(
            value.display_text(),
            "{status: Some Ok (1, ok), metadata: {attempts: [2, 3]}}"
        );
        assert_eq!(
            format!("{value}"),
            "{status: Some Ok (1, ok), metadata: {attempts: [2, 3]}}"
        );
    }

    #[test]
    fn display_handles_deep_signal_nesting_without_recursion() {
        let mut value = RuntimeValue::Int(1);
        for _ in 0..10_000 {
            value = RuntimeValue::Signal(Box::new(value));
        }

        let rendered = format!("{value}");
        assert!(rendered.starts_with("Signal("));
        let suffix = "1".to_owned() + &")".repeat(10_000);
        assert!(rendered.ends_with(&suffix));
    }
}
