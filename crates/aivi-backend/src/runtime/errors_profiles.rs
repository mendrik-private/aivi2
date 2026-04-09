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
    IntrinsicFailed {
        kernel: KernelId,
        expr: KernelExprId,
        value: IntrinsicValue,
        reason: &'static str,
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
            Self::IntrinsicFailed {
                kernel,
                value,
                reason,
                ..
            } => write!(f, "kernel {kernel} intrinsic `{value}` failed: {reason}"),
            Self::UnsupportedDomainMemberCall { kernel, handle, .. } => write!(
                f,
                "kernel {kernel} cannot execute domain member {}.{} in the current backend runtime slice",
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
                "kernel {kernel} contains an inline pipe configuration that the current evaluator cannot execute"
            ),
            Self::UnsupportedInlinePipeSignalSubject { kernel, found, .. } => write!(
                f,
                "kernel {kernel} cannot execute an inline pipe over signal subject `{found}` in the current runtime slice"
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

/// Cached result of the most recent `evaluate_kernel_raw` call.
///
/// Many signal expressions call the same pure kernel with identical arguments many times in a
/// single evaluation pass.  The snake board renderer, for example, calls `snakeHead game.snake`
/// once per cell (480 calls) with the exact same snake list every time.  Storing only the
/// single most-recent result (keyed on kernel + input subject + environment) eliminates the
/// heap allocations for all but the first such call, while avoiding the memory overhead of a
/// general memoization table.
struct LastKernelCall {
    kernel_id: KernelId,
    input_subject: Option<RuntimeValue>,
    environment: Box<[RuntimeValue]>,
    result: RuntimeValue,
    result_layout: LayoutId,
}

/// A lightweight frame in the evaluation trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalFrame {
    pub item: ItemId,
    pub kernel: KernelId,
}

impl fmt::Display for EvalFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "item {} (kernel {})", self.item, self.kernel)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationCallProfile {
    pub calls: u64,
    pub cache_hits: u64,
    pub total_time: Duration,
    pub max_time: Duration,
}

impl EvaluationCallProfile {
    fn record(&mut self, elapsed: Duration, cache_hit: bool) {
        self.calls += 1;
        if cache_hit {
            self.cache_hits += 1;
        }
        self.total_time += elapsed;
        self.max_time = self.max_time.max(elapsed);
    }

    fn merge_from(&mut self, other: &Self) {
        self.calls += other.calls;
        self.cache_hits += other.cache_hits;
        self.total_time += other.total_time;
        self.max_time = self.max_time.max(other.max_time);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KernelEvaluationProfile {
    pub kernels: BTreeMap<KernelId, EvaluationCallProfile>,
    pub items: BTreeMap<ItemId, EvaluationCallProfile>,
}

impl KernelEvaluationProfile {
    fn record_kernel(&mut self, kernel: KernelId, elapsed: Duration, cache_hit: bool) {
        self.kernels
            .entry(kernel)
            .or_default()
            .record(elapsed, cache_hit);
    }

    fn record_item(&mut self, item: ItemId, elapsed: Duration, cache_hit: bool) {
        self.items
            .entry(item)
            .or_default()
            .record(elapsed, cache_hit);
    }

    pub fn merge_from(&mut self, other: &Self) {
        for (kernel, profile) in &other.kernels {
            self.kernels.entry(*kernel).or_default().merge_from(profile);
        }
        for (item, profile) in &other.items {
            self.items.entry(*item).or_default().merge_from(profile);
        }
    }
}

