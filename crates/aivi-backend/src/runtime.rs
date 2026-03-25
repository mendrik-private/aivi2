use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use aivi_hir::{DomainMemberHandle, IntrinsicValue, ItemId as HirItemId, SumConstructorHandle};

use crate::{
    BinaryOperator, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinOrdSubject, BuiltinTerm,
    BuiltinTraversableCarrier, EnvSlotId, InlinePipeConstructor, InlinePipePattern,
    InlinePipePatternKind, InlinePipeStageKind, InlineSubjectId, ItemId, KernelExprId,
    KernelExprKind, KernelId, LayoutId, LayoutKind, PrimitiveType, Program, ProjectionBase,
    SubjectRef, UnaryOperator,
    numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat},
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
pub struct RuntimeSumValue {
    /// The HIR item that defines this sum type.
    ///
    /// # Staleness after recompilation
    ///
    /// This `HirItemId` is assigned at decode/evaluation time and refers to the item identity in
    /// the HIR layer at that moment. After any recompilation that changes HIR structure — for
    /// example, adding, removing, or reordering type definitions — the numeric `HirItemId` may
    /// point at a different item or become invalid entirely. Runtime sum values that were produced
    /// before such a recompile must be re-decoded against the new HIR before they are used in any
    /// context that dispatches on `item` (e.g. structural equality, variant dispatch, serialization).
    pub item: HirItemId,
    pub type_name: Box<str>,
    pub variant_name: Box<str>,
    pub fields: Vec<RuntimeValue>,
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
pub enum RuntimeTaskPlan {
    RandomInt { low: i64, high: i64 },
    RandomBytes { count: i64 },
}

impl fmt::Display for RuntimeTaskPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RandomInt { low, high } => write!(f, "randomInt({low}, {high})"),
            Self::RandomBytes { count } => write!(f, "randomBytes({count})"),
        }
    }
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
    SumConstructor {
        handle: SumConstructorHandle,
        bound_arguments: Vec<RuntimeValue>,
    },
    DomainMember {
        handle: DomainMemberHandle,
        bound_arguments: Vec<RuntimeValue>,
    },
    BuiltinClassMember {
        intrinsic: BuiltinClassMemberIntrinsic,
        bound_arguments: Vec<RuntimeValue>,
    },
    IntrinsicValue {
        value: IntrinsicValue,
        bound_arguments: Vec<RuntimeValue>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeValue {
    Unit,
    Bool(bool),
    Int(i64),
    Float(RuntimeFloat),
    Decimal(RuntimeDecimal),
    BigInt(RuntimeBigInt),
    Text(Box<str>),
    Bytes(Box<[u8]>),
    Tuple(Vec<RuntimeValue>),
    List(Vec<RuntimeValue>),
    // TODO: upgrade to BTreeMap for O(log n) lookup instead of O(n) linear scan.
    // This is blocked on `RuntimeValue` not implementing `Ord`: `RuntimeValue::Float` wraps
    // `RuntimeFloat(f64)`, and `f64` does not implement `Ord` because of NaN. Until a total
    // ordering is defined for all runtime value variants (e.g. by canonicalising NaN or
    // excluding float keys), the map representation must remain a `Vec`.
    Map(Vec<RuntimeMapEntry>),
    Set(Vec<RuntimeValue>),
    Record(Vec<RuntimeRecordField>),
    Sum(RuntimeSumValue),
    OptionNone,
    OptionSome(Box<RuntimeValue>),
    ResultOk(Box<RuntimeValue>),
    ResultErr(Box<RuntimeValue>),
    ValidationValid(Box<RuntimeValue>),
    ValidationInvalid(Box<RuntimeValue>),
    Signal(Box<RuntimeValue>),
    Task(RuntimeTaskPlan),
    SuffixedInteger { raw: Box<str>, suffix: Box<str> },
    Callable(RuntimeCallable),
}

/// Explicit snapshot used when runtime values cross GTK/worker/FFI boundaries.
///
/// Future moving-collector work must not let those boundaries assume that
/// ordinary language values keep stable addresses. This wrapper forces callers to
/// either deep-copy a live runtime value (`from_runtime_copy`) or to explicitly
/// mark an already-owned value as boundary-ready (`from_runtime_owned`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedRuntimeValue(RuntimeValue);

impl DetachedRuntimeValue {
    pub const fn unit() -> Self {
        Self(RuntimeValue::Unit)
    }

    pub fn from_runtime_copy(value: &RuntimeValue) -> Self {
        Self(value.clone())
    }

    pub fn from_runtime_owned(value: RuntimeValue) -> Self {
        Self(value)
    }

    pub const fn as_runtime(&self) -> &RuntimeValue {
        &self.0
    }

    pub fn to_runtime(&self) -> RuntimeValue {
        self.0.clone()
    }

    pub fn into_runtime(self) -> RuntimeValue {
        self.0
    }
}

impl PartialEq<RuntimeValue> for DetachedRuntimeValue {
    fn eq(&self, other: &RuntimeValue) -> bool {
        self.as_runtime() == other
    }
}

impl PartialEq<DetachedRuntimeValue> for RuntimeValue {
    fn eq(&self, other: &DetachedRuntimeValue) -> bool {
        self == other.as_runtime()
    }
}

impl fmt::Display for DetachedRuntimeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_runtime().fmt(f)
    }
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

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(value.to_f64()),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_ref()),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value.as_ref()),
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
                    Self::Float(value) => write!(target, "{value}")?,
                    Self::Decimal(value) => write!(target, "{value}")?,
                    Self::BigInt(value) => write!(target, "{value}")?,
                    Self::Text(value) => target.write_str(value)?,
                    Self::Bytes(value) => write!(target, "<bytes:{}>", value.len())?,
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
                    Self::Sum(value) => {
                        push_sum_value(&mut stack, value);
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
                    Self::Task(task) => write!(target, "<task {task}>")?,
                    Self::SuffixedInteger { raw, suffix } => write!(target, "{raw}{suffix}")?,
                    Self::Callable(callable) => match callable {
                        RuntimeCallable::ItemBody { item, .. } => {
                            write!(target, "<item-body item{item}>")?
                        }
                        RuntimeCallable::BuiltinConstructor { constructor, .. } => {
                            write!(target, "<constructor {constructor}>")?
                        }
                        RuntimeCallable::SumConstructor { handle, .. } => write!(
                            target,
                            "<constructor {}.{}>",
                            handle.type_name, handle.variant_name
                        )?,
                        RuntimeCallable::DomainMember { handle, .. } => write!(
                            target,
                            "<domain-member {}.{}>",
                            handle.domain_name, handle.member_name
                        )?,
                        RuntimeCallable::BuiltinClassMember { intrinsic, .. } => {
                            write!(target, "<builtin-class-member {intrinsic:?}>")?
                        }
                        RuntimeCallable::IntrinsicValue { value, .. } => {
                            write!(target, "<intrinsic-value {value}>")?
                        }
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

fn push_sum_value<'a>(stack: &mut Vec<DisplayFrame<'a>>, value: &'a RuntimeSumValue) {
    match value.fields.as_slice() {
        [] => stack.push(DisplayFrame::BorrowedText(value.variant_name.as_ref())),
        [field] => {
            stack.push(DisplayFrame::Value(field));
            stack.push(DisplayFrame::StaticText(" "));
            stack.push(DisplayFrame::BorrowedText(value.variant_name.as_ref()));
        }
        fields => {
            stack.push(DisplayFrame::StaticText(")"));
            for (index, field) in fields.iter().enumerate().rev() {
                stack.push(DisplayFrame::Value(field));
                if index > 0 {
                    stack.push(DisplayFrame::StaticText(", "));
                }
            }
            stack.push(DisplayFrame::StaticText("("));
            stack.push(DisplayFrame::BorrowedText(value.variant_name.as_ref()));
        }
    }
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
    InvalidIntrinsicArgument {
        kernel: KernelId,
        expr: KernelExprId,
        value: IntrinsicValue,
        index: usize,
        found: RuntimeValue,
    },
    UnsupportedDomainMemberCall {
        kernel: KernelId,
        expr: KernelExprId,
        handle: DomainMemberHandle,
    },
    UnsupportedBuiltinClassMember {
        kernel: KernelId,
        expr: KernelExprId,
        intrinsic: BuiltinClassMemberIntrinsic,
        reason: &'static str,
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
    InvalidBinaryArithmetic {
        kernel: KernelId,
        expr: KernelExprId,
        operator: BinaryOperator,
        left: RuntimeValue,
        right: RuntimeValue,
        reason: &'static str,
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
    InvalidFloatLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    InvalidDecimalLiteral {
        kernel: KernelId,
        expr: KernelExprId,
        raw: Box<str>,
    },
    InvalidBigIntLiteral {
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
            Self::InvalidIntrinsicArgument {
                kernel,
                value,
                index,
                found,
                ..
            } => write!(
                f,
                "kernel {kernel} received invalid argument {} for intrinsic `{value}`: `{found}`",
                index + 1
            ),
            Self::UnsupportedDomainMemberCall { kernel, handle, .. } => write!(
                f,
                "kernel {kernel} reached unresolved domain member {}.{} at runtime",
                handle.domain_name, handle.member_name
            ),
            Self::UnsupportedBuiltinClassMember {
                kernel,
                intrinsic,
                reason,
                ..
            } => write!(
                f,
                "kernel {kernel} cannot evaluate builtin class member `{intrinsic:?}`: {reason}"
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
            Self::InvalidBinaryArithmetic {
                kernel,
                operator,
                left,
                right,
                reason,
                ..
            } => write!(
                f,
                "kernel {kernel} cannot apply binary arithmetic `{operator}` to `{left}` and `{right}`: {reason}"
            ),
            Self::InvalidInterpolationValue { kernel, found, .. } => write!(
                f,
                "kernel {kernel} cannot interpolate callable runtime value `{found}` into text"
            ),
            Self::InvalidIntegerLiteral { kernel, raw, .. } => {
                write!(f, "kernel {kernel} could not parse integer literal `{raw}`")
            }
            Self::InvalidFloatLiteral { kernel, raw, .. } => {
                write!(
                    f,
                    "kernel {kernel} could not parse finite Float literal `{raw}`"
                )
            }
            Self::InvalidDecimalLiteral { kernel, raw, .. } => {
                write!(f, "kernel {kernel} could not parse Decimal literal `{raw}`")
            }
            Self::InvalidBigIntLiteral { kernel, raw, .. } => {
                write!(f, "kernel {kernel} could not parse BigInt literal `{raw}`")
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

    fn evaluate_kernel_raw(
        &mut self,
        kernel_id: KernelId,
        input_subject: Option<&RuntimeValue>,
        environment: &[RuntimeValue],
        globals: &BTreeMap<ItemId, RuntimeValue>,
    ) -> Result<(RuntimeValue, LayoutId), EvaluationError> {
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
        let result = self.evaluate_kernel_raw(kernel, None, &[], globals);
        self.item_stack.remove(&item);
        let (raw_result, expected) = result?;
        let result = match (&item_decl.kind, raw_result) {
            (crate::ItemKind::Signal(_), RuntimeValue::Signal(value))
                if value_matches_layout(self.program, value.as_ref(), expected) =>
            {
                *value
            }
            (_, value) => value,
        };
        if !value_matches_layout(self.program, &result, expected) {
            return Err(EvaluationError::KernelResultLayoutMismatch {
                kernel,
                expected,
                found: result,
            });
        };
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
                        KernelExprKind::SumConstructor(handle) => {
                            values.push(RuntimeValue::Callable(RuntimeCallable::SumConstructor {
                                handle: handle.clone(),
                                bound_arguments: Vec::new(),
                            }))
                        }
                        KernelExprKind::DomainMember(handle) => {
                            values.push(RuntimeValue::Callable(RuntimeCallable::DomainMember {
                                handle: handle.clone(),
                                bound_arguments: Vec::new(),
                            }))
                        }
                        KernelExprKind::BuiltinClassMember(intrinsic) => {
                            values.push(runtime_class_member_value(*intrinsic))
                        }
                        KernelExprKind::Builtin(term) => values.push(map_builtin(*term)),
                        KernelExprKind::IntrinsicValue(value) => {
                            values.push(runtime_intrinsic_value(*value))
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
            let stage_found = current.clone();
            current = coerce_inline_pipe_value(self.program, current, stage.input_layout).ok_or(
                EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: stage.input_layout,
                    found: stage_found,
                },
            )?;
            let mut stage_subjects = inline_subjects.to_vec();
            stage_subjects[stage.subject.index()] = Some(current.clone());
            let result = match &stage.kind {
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
                            if let Some(guard) = arm.guard {
                                let guard = self.evaluate_expr(
                                    kernel_id,
                                    guard,
                                    input_subject,
                                    environment,
                                    &branch_subjects,
                                    globals,
                                )?;
                                match strip_signal(guard) {
                                    RuntimeValue::Bool(true) => {}
                                    RuntimeValue::Bool(false) => continue,
                                    _ => {
                                        return Err(
                                            EvaluationError::UnsupportedInlinePipePattern {
                                                kernel: kernel_id,
                                                expr: expr_id,
                                            },
                                        );
                                    }
                                }
                            }
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
            let result_found = result.clone();
            current = coerce_inline_pipe_value(self.program, result, stage.result_layout).ok_or(
                EvaluationError::KernelResultLayoutMismatch {
                    kernel: kernel_id,
                    expected: stage.result_layout,
                    found: result_found,
                },
            )?;
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
                bound_arguments,
            } => {
                let _ = bound_arguments;
                let _ = arguments;
                Err(EvaluationError::UnsupportedDomainMemberCall {
                    kernel: kernel_id,
                    expr,
                    handle,
                })
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
            BinaryOperator::Multiply => match (&left, &right) {
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                    Ok(RuntimeValue::Int(left * right))
                }
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
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
                _ => Err(EvaluationError::UnsupportedBinary {
                    kernel: kernel_id,
                    expr,
                    operator,
                    left,
                    right,
                }),
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

fn runtime_intrinsic_value(value: IntrinsicValue) -> RuntimeValue {
    RuntimeValue::Callable(RuntimeCallable::IntrinsicValue {
        value,
        bound_arguments: Vec::new(),
    })
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
        IntrinsicValue::RandomBytes => 1,
        IntrinsicValue::RandomInt => 2,
    }
}

fn evaluate_intrinsic_value(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    arguments: Vec<RuntimeValue>,
) -> Result<RuntimeValue, EvaluationError> {
    match (value, arguments.as_slice()) {
        (IntrinsicValue::RandomBytes, [count]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RandomBytes {
                count: expect_intrinsic_i64(kernel, expr, value, 0, count)?,
            }))
        }
        (IntrinsicValue::RandomInt, [low, high]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RandomInt {
                low: expect_intrinsic_i64(kernel, expr, value, 0, low)?,
                high: expect_intrinsic_i64(kernel, expr, value, 1, high)?,
            }))
        }
        _ => unreachable!("intrinsic arity should be enforced before evaluation"),
    }
}

fn builtin_class_member_arity(intrinsic: BuiltinClassMemberIntrinsic) -> usize {
    match intrinsic {
        BuiltinClassMemberIntrinsic::Empty(_) => 0,
        BuiltinClassMemberIntrinsic::Pure(_) => 1,
        BuiltinClassMemberIntrinsic::Bimap(_) | BuiltinClassMemberIntrinsic::Reduce(_) => 3,
        BuiltinClassMemberIntrinsic::StructuralEq
        | BuiltinClassMemberIntrinsic::Compare { .. }
        | BuiltinClassMemberIntrinsic::Append(_)
        | BuiltinClassMemberIntrinsic::Map(_)
        | BuiltinClassMemberIntrinsic::Apply(_)
        | BuiltinClassMemberIntrinsic::Traverse { .. }
        | BuiltinClassMemberIntrinsic::FilterMap(_) => 2,
    }
}

fn pure_applicative_value(
    carrier: BuiltinApplicativeCarrier,
    payload: RuntimeValue,
) -> RuntimeValue {
    match carrier {
        BuiltinApplicativeCarrier::List => RuntimeValue::List(vec![payload]),
        BuiltinApplicativeCarrier::Option => RuntimeValue::OptionSome(Box::new(payload)),
        BuiltinApplicativeCarrier::Result => RuntimeValue::ResultOk(Box::new(payload)),
        BuiltinApplicativeCarrier::Validation => RuntimeValue::ValidationValid(Box::new(payload)),
        BuiltinApplicativeCarrier::Signal => RuntimeValue::Signal(Box::new(payload)),
    }
}

fn wrap_option_in_applicative(
    carrier: BuiltinApplicativeCarrier,
    mapped: RuntimeValue,
) -> Result<RuntimeValue, &'static str> {
    match carrier {
        BuiltinApplicativeCarrier::List => match strip_signal(mapped) {
            RuntimeValue::List(values) => Ok(RuntimeValue::List(
                values
                    .into_iter()
                    .map(|value| RuntimeValue::OptionSome(Box::new(value)))
                    .collect(),
            )),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Option => match strip_signal(mapped) {
            RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
            RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(Box::new(
                RuntimeValue::OptionSome(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Result => match strip_signal(mapped) {
            RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
            RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(Box::new(
                RuntimeValue::OptionSome(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Validation => match strip_signal(mapped) {
            RuntimeValue::ValidationInvalid(error) => Ok(RuntimeValue::ValidationInvalid(error)),
            RuntimeValue::ValidationValid(value) => Ok(RuntimeValue::ValidationValid(Box::new(
                RuntimeValue::OptionSome(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Signal => match mapped {
            RuntimeValue::Signal(value) => Ok(RuntimeValue::Signal(Box::new(
                RuntimeValue::OptionSome(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
    }
}

fn wrap_result_ok_in_applicative(
    carrier: BuiltinApplicativeCarrier,
    mapped: RuntimeValue,
) -> Result<RuntimeValue, &'static str> {
    match carrier {
        BuiltinApplicativeCarrier::List => match strip_signal(mapped) {
            RuntimeValue::List(values) => Ok(RuntimeValue::List(
                values
                    .into_iter()
                    .map(|value| RuntimeValue::ResultOk(Box::new(value)))
                    .collect(),
            )),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Option => match strip_signal(mapped) {
            RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
            RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(Box::new(
                RuntimeValue::ResultOk(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Result => match strip_signal(mapped) {
            RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
            RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(Box::new(
                RuntimeValue::ResultOk(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Validation => match strip_signal(mapped) {
            RuntimeValue::ValidationInvalid(error) => Ok(RuntimeValue::ValidationInvalid(error)),
            RuntimeValue::ValidationValid(value) => Ok(RuntimeValue::ValidationValid(Box::new(
                RuntimeValue::ResultOk(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Signal => match mapped {
            RuntimeValue::Signal(value) => Ok(RuntimeValue::Signal(Box::new(
                RuntimeValue::ResultOk(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
    }
}

fn wrap_validation_valid_in_applicative(
    carrier: BuiltinApplicativeCarrier,
    mapped: RuntimeValue,
) -> Result<RuntimeValue, &'static str> {
    match carrier {
        BuiltinApplicativeCarrier::List => match strip_signal(mapped) {
            RuntimeValue::List(values) => Ok(RuntimeValue::List(
                values
                    .into_iter()
                    .map(|value| RuntimeValue::ValidationValid(Box::new(value)))
                    .collect(),
            )),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Option => match strip_signal(mapped) {
            RuntimeValue::OptionNone => Ok(RuntimeValue::OptionNone),
            RuntimeValue::OptionSome(value) => Ok(RuntimeValue::OptionSome(Box::new(
                RuntimeValue::ValidationValid(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Result => match strip_signal(mapped) {
            RuntimeValue::ResultErr(error) => Ok(RuntimeValue::ResultErr(error)),
            RuntimeValue::ResultOk(value) => Ok(RuntimeValue::ResultOk(Box::new(
                RuntimeValue::ValidationValid(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Validation => match strip_signal(mapped) {
            RuntimeValue::ValidationInvalid(error) => Ok(RuntimeValue::ValidationInvalid(error)),
            RuntimeValue::ValidationValid(value) => Ok(RuntimeValue::ValidationValid(Box::new(
                RuntimeValue::ValidationValid(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
        BuiltinApplicativeCarrier::Signal => match mapped {
            RuntimeValue::Signal(value) => Ok(RuntimeValue::Signal(Box::new(
                RuntimeValue::ValidationValid(value),
            ))),
            _ => Err("traverse expected the mapped value to stay in the target applicative"),
        },
    }
}

fn sequence_traverse_results(
    carrier: BuiltinApplicativeCarrier,
    mapped: Vec<RuntimeValue>,
) -> Result<RuntimeValue, &'static str> {
    match carrier {
        BuiltinApplicativeCarrier::List => {
            let mut accumulated = vec![Vec::new()];
            for value in mapped {
                let RuntimeValue::List(values) = strip_signal(value) else {
                    return Err(
                        "traverse expected the mapped value to stay in the target applicative",
                    );
                };
                let mut next = Vec::new();
                for prefix in &accumulated {
                    for value in &values {
                        let mut candidate = prefix.clone();
                        candidate.push(value.clone());
                        next.push(candidate);
                    }
                }
                accumulated = next;
            }
            Ok(RuntimeValue::List(
                accumulated.into_iter().map(RuntimeValue::List).collect(),
            ))
        }
        BuiltinApplicativeCarrier::Option => {
            let mut collected = Vec::with_capacity(mapped.len());
            for value in mapped {
                match strip_signal(value) {
                    RuntimeValue::OptionNone => return Ok(RuntimeValue::OptionNone),
                    RuntimeValue::OptionSome(value) => collected.push(*value),
                    _ => {
                        return Err(
                            "traverse expected the mapped value to stay in the target applicative",
                        );
                    }
                }
            }
            Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::List(
                collected,
            ))))
        }
        BuiltinApplicativeCarrier::Result => {
            let mut collected = Vec::with_capacity(mapped.len());
            for value in mapped {
                match strip_signal(value) {
                    RuntimeValue::ResultErr(error) => return Ok(RuntimeValue::ResultErr(error)),
                    RuntimeValue::ResultOk(value) => collected.push(*value),
                    _ => {
                        return Err(
                            "traverse expected the mapped value to stay in the target applicative",
                        );
                    }
                }
            }
            Ok(RuntimeValue::ResultOk(Box::new(RuntimeValue::List(
                collected,
            ))))
        }
        BuiltinApplicativeCarrier::Validation => {
            let mut collected = Vec::with_capacity(mapped.len());
            let mut invalid: Option<RuntimeValue> = None;
            for value in mapped {
                match strip_signal(value) {
                    RuntimeValue::ValidationValid(value) => {
                        if invalid.is_none() {
                            collected.push(*value);
                        }
                    }
                    RuntimeValue::ValidationInvalid(error) => {
                        invalid = Some(match invalid {
                            Some(previous) => append_validation_errors(previous, *error)?,
                            None => *error,
                        });
                    }
                    _ => {
                        return Err(
                            "traverse expected the mapped value to stay in the target applicative",
                        );
                    }
                }
            }
            match invalid {
                Some(error) => Ok(RuntimeValue::ValidationInvalid(Box::new(error))),
                None => Ok(RuntimeValue::ValidationValid(Box::new(RuntimeValue::List(
                    collected,
                )))),
            }
        }
        BuiltinApplicativeCarrier::Signal => {
            let mut collected = Vec::with_capacity(mapped.len());
            for value in mapped {
                match value {
                    RuntimeValue::Signal(value) => collected.push(*value),
                    _ => {
                        return Err(
                            "traverse expected the mapped value to stay in the target applicative",
                        );
                    }
                }
            }
            Ok(RuntimeValue::Signal(Box::new(RuntimeValue::List(
                collected,
            ))))
        }
    }
}

fn expect_intrinsic_i64(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<i64, EvaluationError> {
    match strip_signal(argument.clone()) {
        RuntimeValue::Int(found) => Ok(found),
        found => Err(EvaluationError::InvalidIntrinsicArgument {
            kernel,
            expr,
            value,
            index,
            found: found.clone(),
        }),
    }
}

fn expect_arity<const N: usize>(
    arguments: Vec<RuntimeValue>,
) -> Result<[RuntimeValue; N], &'static str> {
    arguments
        .try_into()
        .map_err(|_| "applied argument count did not match the builtin class member arity")
}

fn ordering_value(ordering_item: HirItemId, ordering: std::cmp::Ordering) -> RuntimeValue {
    let variant_name = match ordering {
        std::cmp::Ordering::Less => "Less",
        std::cmp::Ordering::Equal => "Equal",
        std::cmp::Ordering::Greater => "Greater",
    };
    RuntimeValue::Sum(RuntimeSumValue {
        item: ordering_item,
        type_name: "Ordering".into(),
        variant_name: variant_name.into(),
        fields: Vec::new(),
    })
}

fn ordering_rank(variant_name: &str) -> u8 {
    match variant_name {
        "Less" => 0,
        "Equal" => 1,
        "Greater" => 2,
        _ => 3,
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
        (LayoutKind::Primitive(PrimitiveType::Float), RuntimeValue::Float(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Decimal), RuntimeValue::Decimal(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::BigInt), RuntimeValue::BigInt(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Text), RuntimeValue::Text(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Bytes), RuntimeValue::Bytes(_)) => true,
        (LayoutKind::Primitive(PrimitiveType::Task), RuntimeValue::Task(_)) => true,
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
        (LayoutKind::Arrow { .. }, RuntimeValue::Callable(_)) => true,
        (LayoutKind::AnonymousDomain { .. }, RuntimeValue::SuffixedInteger { .. })
        | (LayoutKind::Domain { .. }, RuntimeValue::SuffixedInteger { .. }) => true,
        (LayoutKind::Opaque { name, .. }, RuntimeValue::Sum(value)) => {
            name.as_ref() == value.type_name.as_ref()
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
        (RuntimeValue::Sum(left), RuntimeValue::Sum(right)) => {
            if left.item != right.item
                || left.variant_name != right.variant_name
                || left.fields.len() != right.fields.len()
            {
                false
            } else {
                let mut equal = true;
                for (left, right) in left.fields.iter().zip(right.fields.iter()) {
                    equal &= structural_eq(kernel, expr, left, right)?;
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

fn coerce_runtime_value(
    program: &Program,
    value: RuntimeValue,
    layout: LayoutId,
) -> Result<RuntimeValue, RuntimeValue> {
    if value_matches_layout(program, &value, layout) {
        return Ok(value);
    }
    if let RuntimeValue::Signal(inner) = &value {
        let payload = inner.as_ref().clone();
        if value_matches_layout(program, &payload, layout) {
            return Ok(payload);
        }
    }
    let Some(layout) = program.layouts().get(layout) else {
        return Err(value);
    };
    let LayoutKind::Signal { element } = &layout.kind else {
        return Err(value);
    };
    if value_matches_layout(program, &value, *element) {
        Ok(RuntimeValue::Signal(Box::new(value)))
    } else {
        Err(value)
    }
}

fn coerce_inline_pipe_value(
    program: &Program,
    value: RuntimeValue,
    layout: LayoutId,
) -> Option<RuntimeValue> {
    coerce_runtime_value(program, value, layout).ok()
}

fn strip_signal(value: RuntimeValue) -> RuntimeValue {
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

#[cfg(test)]
mod tests {
    use aivi_hir::{ItemId as HirItemId, SumConstructorHandle};

    use super::{
        DetachedRuntimeValue, RuntimeMapEntry, RuntimeRecordField, RuntimeSumValue, RuntimeValue,
        append_validation_errors,
    };

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

    #[test]
    fn display_formats_user_sum_values() {
        let value = RuntimeValue::Sum(RuntimeSumValue {
            item: HirItemId::from_raw(3),
            type_name: "ResultLike".into(),
            variant_name: "Pair".into(),
            fields: vec![RuntimeValue::Int(1), RuntimeValue::Text("ok".into())],
        });

        assert_eq!(value.display_text(), "Pair(1, ok)");
    }

    #[test]
    fn display_formats_user_sum_constructors() {
        let value = RuntimeValue::Callable(super::RuntimeCallable::SumConstructor {
            handle: SumConstructorHandle {
                item: HirItemId::from_raw(3),
                type_name: "Status".into(),
                variant_name: "Ready".into(),
                field_count: 0,
            },
            bound_arguments: Vec::new(),
        });

        assert_eq!(format!("{value}"), "<constructor Status.Ready>");
    }

    #[test]
    fn validation_error_accumulation_appends_non_empty_payloads() {
        let left = RuntimeValue::Sum(RuntimeSumValue {
            item: HirItemId::from_raw(11),
            type_name: "NonEmptyList".into(),
            variant_name: "NonEmptyList".into(),
            fields: vec![
                RuntimeValue::Text("missing name".into()),
                RuntimeValue::List(Vec::new()),
            ],
        });
        let right = RuntimeValue::Sum(RuntimeSumValue {
            item: HirItemId::from_raw(11),
            type_name: "NonEmptyList".into(),
            variant_name: "NonEmptyList".into(),
            fields: vec![
                RuntimeValue::Text("missing email".into()),
                RuntimeValue::List(vec![RuntimeValue::Text("missing age".into())]),
            ],
        });

        let accumulated = append_validation_errors(left, right)
            .expect("non-empty validation errors should append");

        assert_eq!(
            accumulated,
            RuntimeValue::Sum(RuntimeSumValue {
                item: HirItemId::from_raw(11),
                type_name: "NonEmptyList".into(),
                variant_name: "NonEmptyList".into(),
                fields: vec![
                    RuntimeValue::Text("missing name".into()),
                    RuntimeValue::List(vec![
                        RuntimeValue::Text("missing email".into()),
                        RuntimeValue::Text("missing age".into()),
                    ]),
                ],
            })
        );
    }

    #[test]
    fn detached_runtime_values_copy_text_storage_at_boundary() {
        let original = RuntimeValue::Signal(Box::new(RuntimeValue::Text("hello".into())));
        let detached = DetachedRuntimeValue::from_runtime_copy(&original);

        let RuntimeValue::Signal(original_inner) = &original else {
            panic!("expected wrapped signal value")
        };
        let RuntimeValue::Text(original_text) = original_inner.as_ref() else {
            panic!("expected wrapped text payload")
        };
        let RuntimeValue::Signal(detached_inner) = detached.as_runtime() else {
            panic!("expected detached wrapped signal value")
        };
        let RuntimeValue::Text(detached_text) = detached_inner.as_ref() else {
            panic!("expected detached wrapped text payload")
        };

        assert_eq!(detached, original);
        assert_ne!(
            original_text.as_ptr(),
            detached_text.as_ptr(),
            "detaching must copy boundary text storage instead of preserving addresses"
        );
    }
}
