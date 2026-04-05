use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::Hash,
};

use indexmap::IndexMap;

use aivi_hir::{DomainMemberHandle, IntrinsicValue, ItemId as HirItemId, SumConstructorHandle};

use crate::{
    BinaryOperator, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject,
    BuiltinTerm, BuiltinTraversableCarrier, EnvSlotId, InlinePipeConstructor, InlinePipePattern,
    InlinePipePatternKind, InlinePipeStageKind, InlineSubjectId, ItemId, KernelExprId,
    KernelExprKind, KernelId, LayoutId, LayoutKind, PrimitiveType, Program, ProjectionBase,
    SubjectRef, UnaryOperator,
    numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeRecordField {
    pub label: Box<str>,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeMapEntry {
    pub key: RuntimeValue,
    pub value: RuntimeValue,
}

/// Ordered runtime map storage backed by an `IndexMap`.
///
/// Keys must implement `Hash + Eq` so that lookups are O(1) rather than
/// O(n). Insertion order is preserved exactly as written in the source,
/// satisfying the display and serialisation invariant that `{b: 2, a: 1}`
/// prints with `b` before `a`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeMap(IndexMap<RuntimeValue, RuntimeValue>);

impl RuntimeMap {
    /// Build a map from a list of entries, preserving insertion order.
    ///
    /// Duplicate keys are silently overwritten with the last value, matching
    /// the evaluation-time behaviour for map literals with repeated keys.
    pub fn from_entries(entries: Vec<RuntimeMapEntry>) -> Self {
        let mut map = IndexMap::with_capacity(entries.len());
        for entry in entries {
            map.insert(entry.key, entry.value);
        }
        Self(map)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over key-value pairs in insertion order.
    pub fn iter(&self) -> indexmap::map::Iter<'_, RuntimeValue, RuntimeValue> {
        self.0.iter()
    }

    /// Look up a value by key in O(1) time.
    pub fn get(&self, key: &RuntimeValue) -> Option<&RuntimeValue> {
        self.0.get(key)
    }
}

impl std::hash::Hash for RuntimeMap {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for (k, v) in &self.0 {
            k.hash(state);
            v.hash(state);
        }
    }
}

impl<'a> IntoIterator for &'a RuntimeMap {
    type Item = (&'a RuntimeValue, &'a RuntimeValue);
    type IntoIter = indexmap::map::Iter<'a, RuntimeValue, RuntimeValue>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeConstructor {
    Some,
    Ok,
    Err,
    Valid,
    Invalid,
}

/// Backend-owned DB task plans stay separate from `RuntimeTaskPlan` until executor integration
/// lands. This keeps the representation explicit without forcing runtime/CLI wiring in this slice.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeDbTaskPlan {
    Query(RuntimeDbQueryPlan),
    Commit(RuntimeDbCommitPlan),
}

/// Path-backed database identity extracted from the surface `Connection` record.
///
/// The `database` text must already be normalized so equality and change invalidation use the same
/// canonical key.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeDbConnection {
    pub database: Box<str>,
}

/// One SQL statement plus its bound arguments.
///
/// Argument order is significant and preserves the lowering order so later execution can bind
/// placeholders deterministically without re-inspecting source syntax.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeDbStatement {
    pub sql: Box<str>,
    pub arguments: Vec<RuntimeValue>,
}

/// Read-only DB work.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeDbQueryPlan {
    pub connection: RuntimeDbConnection,
    pub statement: RuntimeDbStatement,
}

/// Transactional DB work whose successful commit must invalidate explicit table keys.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeDbCommitPlan {
    pub connection: RuntimeDbConnection,
    pub statements: Vec<RuntimeDbStatement>,
    pub changed_tables: BTreeSet<Box<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeNamedValue {
    pub name: Box<str>,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeCustomCapabilityCommandPlan {
    pub provider_key: Box<str>,
    pub command: Box<str>,
    pub provider_arguments: Box<[RuntimeNamedValue]>,
    pub options: Box<[RuntimeNamedValue]>,
    pub arguments: Box<[RuntimeNamedValue]>,
}

impl fmt::Display for RuntimeDbTaskPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query(plan) => write!(f, "db.{plan}"),
            Self::Commit(plan) => write!(f, "db.{plan}"),
        }
    }
}

impl fmt::Display for RuntimeDbConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "db.connection({})", self.database)
    }
}

impl fmt::Display for RuntimeDbStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sql({}", self.sql)?;
        if !self.arguments.is_empty() {
            f.write_str("; args: [")?;
            for (index, argument) in self.arguments.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{argument}")?;
            }
            f.write_str("]")?;
        }
        f.write_str(")")
    }
}

impl fmt::Display for RuntimeDbQueryPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query({}, {})", self.connection, self.statement)
    }
}

impl fmt::Display for RuntimeDbCommitPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "commit({}, [", self.connection)?;
        for (index, statement) in self.statements.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{statement}")?;
        }
        f.write_str("]; changes: [")?;
        for (index, table) in self.changed_tables.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            f.write_str(table)?;
        }
        f.write_str("])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeTaskPlan {
    Pure {
        value: Box<RuntimeValue>,
    },
    RandomInt {
        low: i64,
        high: i64,
    },
    RandomBytes {
        count: i64,
    },
    StdoutWrite {
        text: Box<str>,
    },
    StderrWrite {
        text: Box<str>,
    },
    FsWriteText {
        path: Box<str>,
        text: Box<str>,
    },
    FsWriteBytes {
        path: Box<str>,
        bytes: Box<[u8]>,
    },
    FsCreateDirAll {
        path: Box<str>,
    },
    FsDeleteFile {
        path: Box<str>,
    },
    FsReadText {
        path: Box<str>,
    },
    FsReadDir {
        path: Box<str>,
    },
    FsExists {
        path: Box<str>,
    },
    FsReadBytes {
        path: Box<str>,
    },
    FsRename {
        from: Box<str>,
        to: Box<str>,
    },
    FsCopy {
        from: Box<str>,
        to: Box<str>,
    },
    FsDeleteDir {
        path: Box<str>,
    },
    JsonValidate {
        json: Box<str>,
    },
    JsonGet {
        json: Box<str>,
        key: Box<str>,
    },
    JsonAt {
        json: Box<str>,
        index: i64,
    },
    JsonKeys {
        json: Box<str>,
    },
    JsonPretty {
        json: Box<str>,
    },
    JsonMinify {
        json: Box<str>,
    },
    // Time task plans
    TimeNowMs,
    TimeMonotonicMs,
    TimeFormat {
        epoch_ms: i64,
        pattern: Box<str>,
    },
    TimeParse {
        text: Box<str>,
        pattern: Box<str>,
    },
    // Env task plans
    EnvGet {
        name: Box<str>,
    },
    EnvList {
        prefix: Box<str>,
    },
    // Log task plans
    LogEmit {
        level: Box<str>,
        message: Box<str>,
    },
    LogEmitContext {
        level: Box<str>,
        message: Box<str>,
        context: Box<[(Box<str>, Box<str>)]>,
    },
    // Random float task plan
    RandomFloat,
    // Regex task plans
    RegexIsMatch {
        pattern: Box<str>,
        text: Box<str>,
    },
    RegexFind {
        pattern: Box<str>,
        text: Box<str>,
    },
    RegexFindText {
        pattern: Box<str>,
        text: Box<str>,
    },
    RegexFindAll {
        pattern: Box<str>,
        text: Box<str>,
    },
    RegexReplace {
        pattern: Box<str>,
        replacement: Box<str>,
        text: Box<str>,
    },
    RegexReplaceAll {
        pattern: Box<str>,
        replacement: Box<str>,
        text: Box<str>,
    },
    // HTTP task plans (run on worker thread via ureq)
    HttpGet {
        url: Box<str>,
    },
    HttpGetBytes {
        url: Box<str>,
    },
    HttpGetStatus {
        url: Box<str>,
    },
    HttpPost {
        url: Box<str>,
        content_type: Box<str>,
        body: Box<str>,
    },
    HttpPut {
        url: Box<str>,
        content_type: Box<str>,
        body: Box<str>,
    },
    HttpDelete {
        url: Box<str>,
    },
    HttpHead {
        url: Box<str>,
    },
    HttpPostJson {
        url: Box<str>,
        body: Box<str>,
    },
    CustomCapabilityCommand(RuntimeCustomCapabilityCommandPlan),
    /// Deferred map: execute `inner`, then apply `function` to the result and wrap in `Pure`.
    Map {
        function: Box<RuntimeValue>,
        inner: Box<RuntimeTaskPlan>,
    },
    /// Deferred apply: execute both tasks, apply the function-result to the value-result.
    Apply {
        function_task: Box<RuntimeTaskPlan>,
        value_task: Box<RuntimeTaskPlan>,
    },
    /// Deferred chain: execute `inner`, apply `function` (which returns a `Task`), execute that.
    Chain {
        function: Box<RuntimeValue>,
        inner: Box<RuntimeTaskPlan>,
    },
    /// Deferred join: execute `outer` (which produces a `Task`), then execute that inner task.
    Join {
        outer: Box<RuntimeTaskPlan>,
    },
}

impl fmt::Display for RuntimeTaskPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pure { value } => write!(f, "pure({value})"),
            Self::RandomInt { low, high } => write!(f, "randomInt({low}, {high})"),
            Self::RandomBytes { count } => write!(f, "randomBytes({count})"),
            Self::StdoutWrite { text } => write!(f, "stdoutWrite({text})"),
            Self::StderrWrite { text } => write!(f, "stderrWrite({text})"),
            Self::FsWriteText { path, .. } => write!(f, "writeText({path})"),
            Self::FsWriteBytes { path, .. } => write!(f, "writeBytes({path})"),
            Self::FsCreateDirAll { path } => write!(f, "createDirAll({path})"),
            Self::FsDeleteFile { path } => write!(f, "deleteFile({path})"),
            Self::FsReadText { path } => write!(f, "readText({path})"),
            Self::FsReadDir { path } => write!(f, "readDir({path})"),
            Self::FsExists { path } => write!(f, "exists({path})"),
            Self::FsReadBytes { path } => write!(f, "readBytes({path})"),
            Self::FsRename { from, to } => write!(f, "rename({from}, {to})"),
            Self::FsCopy { from, to } => write!(f, "copy({from}, {to})"),
            Self::FsDeleteDir { path } => write!(f, "deleteDir({path})"),
            Self::JsonValidate { json } => write!(f, "json.validate({json})"),
            Self::JsonGet { json, key } => write!(f, "json.get({json}, {key})"),
            Self::JsonAt { json, index } => write!(f, "json.at({json}, {index})"),
            Self::JsonKeys { json } => write!(f, "json.keys({json})"),
            Self::JsonPretty { json } => write!(f, "json.pretty({json})"),
            Self::JsonMinify { json } => write!(f, "json.minify({json})"),
            Self::TimeNowMs => f.write_str("time.nowMs"),
            Self::TimeMonotonicMs => f.write_str("time.monotonicMs"),
            Self::TimeFormat { epoch_ms, pattern } => {
                write!(f, "time.format({epoch_ms}, {pattern})")
            }
            Self::TimeParse { text, pattern } => write!(f, "time.parse({text}, {pattern})"),
            Self::EnvGet { name } => write!(f, "env.get({name})"),
            Self::EnvList { prefix } => write!(f, "env.list({prefix})"),
            Self::LogEmit { level, message } => write!(f, "log.emit({level}, {message})"),
            Self::LogEmitContext { level, message, .. } => {
                write!(f, "log.emitContext({level}, {message})")
            }
            Self::RandomFloat => f.write_str("random.randomFloat"),
            Self::RegexIsMatch { pattern, text } => write!(f, "regex.isMatch({pattern}, {text})"),
            Self::RegexFind { pattern, text } => write!(f, "regex.find({pattern}, {text})"),
            Self::RegexFindText { pattern, text } => {
                write!(f, "regex.findText({pattern}, {text})")
            }
            Self::RegexFindAll { pattern, text } => write!(f, "regex.findAll({pattern}, {text})"),
            Self::RegexReplace {
                pattern,
                replacement,
                text,
            } => {
                write!(f, "regex.replace({pattern}, {replacement}, {text})")
            }
            Self::RegexReplaceAll {
                pattern,
                replacement,
                text,
            } => {
                write!(f, "regex.replaceAll({pattern}, {replacement}, {text})")
            }
            Self::HttpGet { url } => write!(f, "http.get({url})"),
            Self::HttpGetBytes { url } => write!(f, "http.getBytes({url})"),
            Self::HttpGetStatus { url } => write!(f, "http.getStatus({url})"),
            Self::HttpPost { url, .. } => write!(f, "http.post({url})"),
            Self::HttpPut { url, .. } => write!(f, "http.put({url})"),
            Self::HttpDelete { url } => write!(f, "http.delete({url})"),
            Self::HttpHead { url } => write!(f, "http.head({url})"),
            Self::HttpPostJson { url, .. } => write!(f, "http.postJson({url})"),
            Self::CustomCapabilityCommand(plan) => {
                write!(f, "{}.{}", plan.provider_key, plan.command)
            }
            Self::Map { .. } => f.write_str("task.map(...)"),
            Self::Apply { .. } => f.write_str("task.apply(...)"),
            Self::Chain { .. } => f.write_str("task.chain(...)"),
            Self::Join { .. } => f.write_str("task.join(...)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
        parameters: Vec<LayoutId>,
        result: LayoutId,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    // Runtime maps intentionally preserve source entry order. The dedicated
    // wrapper keeps that invariant explicit while lookup remains linear until
    // the runtime defines a total key ordering for all `RuntimeValue` variants.
    Map(RuntimeMap),
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
    DbTask(RuntimeDbTaskPlan),
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
                    Self::DbTask(task) => write!(target, "<task {task}>")?,
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

fn push_map_entries<'a>(stack: &mut Vec<DisplayFrame<'a>>, entries: &'a RuntimeMap) {
    stack.push(DisplayFrame::StaticText("}"));
    for (index, (key, value)) in entries.iter().enumerate().rev() {
        stack.push(DisplayFrame::Value(value));
        stack.push(DisplayFrame::StaticText(": "));
        stack.push(DisplayFrame::Value(key));
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

pub struct KernelEvaluator<'a> {
    program: &'a Program,
    item_cache: BTreeMap<ItemId, RuntimeValue>,
    item_stack: BTreeSet<ItemId>,
    /// Ordered evaluation trace: items visited during the current evaluation,
    /// in the order they were first entered. Used for error rendering.
    eval_trace: Vec<EvalFrame>,
    last_kernel_call: Option<LastKernelCall>,
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
        }
    }

    pub fn program(&self) -> &'a Program {
        self.program
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
        if let Some(ref last) = self.last_kernel_call {
            if last.kernel_id == kernel_id
                && last.input_subject.as_ref() == input_subject
                && last.environment.as_ref() == environment
            {
                return Ok((last.result.clone(), last.result_layout));
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
        self.eval_trace.push(EvalFrame { item, kernel });
        let result = self.evaluate_kernel_raw(kernel, None, &[], globals);
        self.item_stack.remove(&item);
        let (raw_result, expected) = match result {
            Ok(v) => {
                self.eval_trace.pop();
                v
            }
            Err(e) => return Err(e),
        };
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
            ("value" | "unwrap", [argument])
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
                    (
                        RuntimeValue::Task(function_plan),
                        RuntimeValue::Task(value_plan),
                    ) => match (function_plan, value_plan) {
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
                    },
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

/// Return an XDG base directory: use the env var if set and non-empty, otherwise `$HOME/fallback`.
fn xdg_dir(env_var: &str, fallback: &str) -> String {
    if let Ok(val) = std::env::var(env_var) {
        if !val.is_empty() {
            return val;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_owned());
    format!("{home}/{fallback}")
}

/// Return a colon-separated XDG search path as a list. Falls back to `defaults` when the env var
/// is absent or empty.
fn xdg_search_dirs(env_var: &str, defaults: &[&str]) -> Vec<RuntimeValue> {
    let raw = std::env::var(env_var).unwrap_or_default();
    if raw.is_empty() {
        defaults
            .iter()
            .map(|s| RuntimeValue::Text((*s).into()))
            .collect()
    } else {
        raw.split(':')
            .filter(|s| !s.is_empty())
            .map(|s| RuntimeValue::Text(s.into()))
            .collect()
    }
}

fn evaluate_intrinsic_value(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    arguments: Vec<RuntimeValue>,
) -> Result<RuntimeValue, EvaluationError> {
    match &value {
        IntrinsicValue::TupleConstructor { arity } => {
            debug_assert_eq!(arguments.len(), *arity);
            return Ok(RuntimeValue::Tuple(arguments));
        }
        IntrinsicValue::CustomCapabilityCommand(spec) => {
            return Ok(RuntimeValue::Task(
                RuntimeTaskPlan::CustomCapabilityCommand(runtime_custom_capability_command_plan(
                    arguments, spec,
                )),
            ));
        }
        _ => {}
    }
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
        (IntrinsicValue::StdoutWrite, [text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::StdoutWrite {
                text: expect_intrinsic_text(kernel, expr, value, 0, text)?,
            }))
        }
        (IntrinsicValue::StderrWrite, [text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::StderrWrite {
                text: expect_intrinsic_text(kernel, expr, value, 0, text)?,
            }))
        }
        (IntrinsicValue::FsWriteText, [path, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsWriteText {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
                text: expect_intrinsic_text(kernel, expr, value, 1, text)?,
            }))
        }
        (IntrinsicValue::FsWriteBytes, [path, bytes]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsWriteBytes {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
                bytes: expect_intrinsic_bytes(kernel, expr, value, 1, bytes)?,
            }))
        }
        (IntrinsicValue::FsCreateDirAll, [path]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsCreateDirAll {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
            }))
        }
        (IntrinsicValue::FsDeleteFile, [path]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsDeleteFile {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
            }))
        }
        (IntrinsicValue::DbParamBool, [argument]) => Ok(runtime_db_param(
            "bool",
            "bool",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamInt, [argument]) => Ok(runtime_db_param(
            "int",
            "int",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamFloat, [argument]) => Ok(runtime_db_param(
            "float",
            "float",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamDecimal, [argument]) => Ok(runtime_db_param(
            "decimal",
            "decimal",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamBigInt, [argument]) => Ok(runtime_db_param(
            "bigInt",
            "bigInt",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamText, [argument]) => Ok(runtime_db_param(
            "text",
            "text",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbParamBytes, [argument]) => Ok(runtime_db_param(
            "bytes",
            "bytes",
            strip_signal(argument.clone()),
        )),
        (IntrinsicValue::DbStatement, [sql, arguments]) => {
            let sql = expect_intrinsic_text(kernel, expr, value, 0, sql)?;
            let arguments = match strip_signal(arguments.clone()) {
                RuntimeValue::List(arguments) => arguments,
                found => {
                    return Err(EvaluationError::InvalidIntrinsicArgument {
                        kernel,
                        expr,
                        value,
                        index: 1,
                        found,
                    });
                }
            };
            Ok(runtime_db_statement(sql, arguments))
        }
        (IntrinsicValue::DbQuery, [connection, statement]) => Ok(RuntimeValue::DbTask(
            RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
                connection: expect_intrinsic_db_connection(kernel, expr, value, 0, connection)?,
                statement: expect_intrinsic_db_statement(kernel, expr, value, 1, statement)?,
            }),
        )),
        (IntrinsicValue::DbCommit, [connection, changed_tables, statements]) => Ok(
            RuntimeValue::DbTask(RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
                connection: expect_intrinsic_db_connection(kernel, expr, value, 0, connection)?,
                statements: expect_intrinsic_db_statement_list(kernel, expr, value, 2, statements)?,
                changed_tables: expect_intrinsic_text_list(kernel, expr, value, 1, changed_tables)?
                    .into_iter()
                    .collect(),
            })),
        ),
        // Float math intrinsics — pure functions, return directly
        (IntrinsicValue::FloatFloor, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.floor())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatFloor,
                    reason: "floor result is not finite".into(),
                })
        }
        (IntrinsicValue::FloatCeil, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.ceil())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatCeil,
                    reason: "ceil result is not finite".into(),
                })
        }
        (IntrinsicValue::FloatRound, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.round())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatRound,
                    reason: "round result is not finite".into(),
                })
        }
        (IntrinsicValue::FloatSqrt, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.sqrt())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatSqrt,
                    reason: "sqrt of negative number",
                })
        }
        (IntrinsicValue::FloatAbs, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.abs())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatAbs,
                    reason: "abs result is not finite".into(),
                })
        }
        (IntrinsicValue::FloatToInt, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::Int(f as i64))
        }
        (IntrinsicValue::FloatFromInt, [n]) => {
            let i = expect_intrinsic_i64(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(i as f64)
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatFromInt,
                    reason: "int-to-float result is not finite".into(),
                })
        }
        (IntrinsicValue::FloatToText, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::Text(f.to_string().into()))
        }
        (IntrinsicValue::FloatParseText, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            let result = s.parse::<f64>().ok().and_then(RuntimeFloat::new);
            match result {
                Some(f) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Float(f)))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        // FS read intrinsics
        (IntrinsicValue::FsReadText, [path]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsReadText {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
            }))
        }
        (IntrinsicValue::FsReadDir, [path]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::FsReadDir {
            path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
        })),
        (IntrinsicValue::FsExists, [path]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::FsExists {
            path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
        })),
        (IntrinsicValue::FsReadBytes, [path]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsReadBytes {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
            }))
        }
        (IntrinsicValue::FsRename, [from, to]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsRename {
                from: expect_intrinsic_text(kernel, expr, value, 0, from)?,
                to: expect_intrinsic_text(kernel, expr, value, 1, to)?,
            }))
        }
        (IntrinsicValue::FsCopy, [from, to]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::FsCopy {
            from: expect_intrinsic_text(kernel, expr, value, 0, from)?,
            to: expect_intrinsic_text(kernel, expr, value, 1, to)?,
        })),
        (IntrinsicValue::FsDeleteDir, [path]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::FsDeleteDir {
                path: expect_intrinsic_text(kernel, expr, value, 0, path)?,
            }))
        }
        (IntrinsicValue::PathParent, [path]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            let p = std::path::Path::new(&*s);
            Ok(p.parent()
                .and_then(|p| p.to_str())
                .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::PathFilename, [path]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            let p = std::path::Path::new(&*s);
            Ok(p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::PathStem, [path]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            let p = std::path::Path::new(&*s);
            Ok(p.file_stem()
                .and_then(|n| n.to_str())
                .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::PathExtension, [path]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            let p = std::path::Path::new(&*s);
            Ok(p.extension()
                .and_then(|n| n.to_str())
                .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::PathJoin, [base, segment]) => {
            let b = expect_intrinsic_text(kernel, expr, value, 0, base)?;
            let s = expect_intrinsic_text(kernel, expr, value, 1, segment)?;
            let joined = std::path::Path::new(&*b).join(&*s);
            Ok(RuntimeValue::Text(
                joined.to_string_lossy().into_owned().into(),
            ))
        }
        (IntrinsicValue::PathIsAbsolute, [path]) => {
            let p = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            Ok(RuntimeValue::Bool(std::path::Path::new(&*p).is_absolute()))
        }
        (IntrinsicValue::PathNormalize, [path]) => {
            let p = expect_intrinsic_text(kernel, expr, value, 0, path)?;
            // Lexical normalization only — resolves `.` and `..` without I/O.
            let mut components: Vec<&str> = Vec::new();
            for component in std::path::Path::new(&*p).components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        components.pop();
                    }
                    other => {
                        if let Some(s) = other.as_os_str().to_str() {
                            components.push(s);
                        }
                    }
                }
            }
            Ok(RuntimeValue::Text(components.join("/").into()))
        }
        (IntrinsicValue::BytesEmpty, []) => Ok(RuntimeValue::Bytes(Box::new([]))),
        (IntrinsicValue::BytesLength, [b]) => {
            let bytes = expect_intrinsic_bytes(kernel, expr, value, 0, b)?;
            Ok(RuntimeValue::Int(bytes.len() as i64))
        }
        (IntrinsicValue::BytesGet, [idx, b]) => {
            let i = expect_intrinsic_i64(kernel, expr, value, 0, idx)?;
            let bytes = expect_intrinsic_bytes(kernel, expr, value, 1, b)?;
            Ok(usize::try_from(i)
                .ok()
                .and_then(|i| bytes.get(i))
                .map(|&byte| RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(byte as i64))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::BytesSlice, [from, to, b]) => {
            let start = expect_intrinsic_i64(kernel, expr, value, 0, from)?;
            let end = expect_intrinsic_i64(kernel, expr, value, 1, to)?;
            let bytes = expect_intrinsic_bytes(kernel, expr, value, 2, b)?;
            let start = (start as usize).min(bytes.len());
            let end = (end as usize).min(bytes.len());
            let end = end.max(start);
            Ok(RuntimeValue::Bytes(bytes[start..end].into()))
        }
        (IntrinsicValue::BytesAppend, [a, b]) => {
            let left = expect_intrinsic_bytes(kernel, expr, value, 0, a)?;
            let right = expect_intrinsic_bytes(kernel, expr, value, 1, b)?;
            let mut combined = left.to_vec();
            combined.extend_from_slice(&*right);
            Ok(RuntimeValue::Bytes(combined.into()))
        }
        (IntrinsicValue::BytesFromText, [t]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, t)?;
            Ok(RuntimeValue::Bytes(text.as_bytes().into()))
        }
        (IntrinsicValue::BytesToText, [b]) => {
            let bytes = expect_intrinsic_bytes(kernel, expr, value, 0, b)?;
            Ok(std::str::from_utf8(&*bytes)
                .ok()
                .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
                .unwrap_or(RuntimeValue::OptionNone))
        }
        (IntrinsicValue::BytesRepeat, [byte_val, count]) => {
            let b = expect_intrinsic_i64(kernel, expr, value, 0, byte_val)?;
            let n = expect_intrinsic_i64(kernel, expr, value, 1, count)?;
            let byte = (b.clamp(0, 255)) as u8;
            let n = (n.max(0)) as usize;
            Ok(RuntimeValue::Bytes(vec![byte; n].into()))
        }
        (IntrinsicValue::JsonValidate, [json]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonValidate {
                json: text,
            }))
        }
        (IntrinsicValue::JsonGet, [json, key]) => {
            let j = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            let k = expect_intrinsic_text(kernel, expr, value, 1, key)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonGet {
                json: j,
                key: k,
            }))
        }
        (IntrinsicValue::JsonAt, [json, index]) => {
            let j = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            let i = expect_intrinsic_i64(kernel, expr, value, 1, index)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonAt {
                json: j,
                index: i,
            }))
        }
        (IntrinsicValue::JsonKeys, [json]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonKeys { json: text }))
        }
        (IntrinsicValue::JsonPretty, [json]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonPretty {
                json: text,
            }))
        }
        (IntrinsicValue::JsonMinify, [json]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, json)?;
            Ok(RuntimeValue::Task(RuntimeTaskPlan::JsonMinify {
                json: text,
            }))
        }
        (IntrinsicValue::XdgDataHome, []) => {
            let path = xdg_dir("XDG_DATA_HOME", ".local/share");
            Ok(RuntimeValue::Text(path.into()))
        }
        (IntrinsicValue::XdgConfigHome, []) => {
            let path = xdg_dir("XDG_CONFIG_HOME", ".config");
            Ok(RuntimeValue::Text(path.into()))
        }
        (IntrinsicValue::XdgCacheHome, []) => {
            let path = xdg_dir("XDG_CACHE_HOME", ".cache");
            Ok(RuntimeValue::Text(path.into()))
        }
        (IntrinsicValue::XdgStateHome, []) => {
            let path = xdg_dir("XDG_STATE_HOME", ".local/state");
            Ok(RuntimeValue::Text(path.into()))
        }
        (IntrinsicValue::XdgRuntimeDir, []) => Ok(std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|s| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(s.into()))))
            .unwrap_or(RuntimeValue::OptionNone)),
        (IntrinsicValue::XdgDataDirs, []) => {
            let dirs = xdg_search_dirs("XDG_DATA_DIRS", &["/usr/local/share", "/usr/share"]);
            Ok(RuntimeValue::List(dirs))
        }
        (IntrinsicValue::XdgConfigDirs, []) => {
            let dirs = xdg_search_dirs("XDG_CONFIG_DIRS", &["/etc/xdg"]);
            Ok(RuntimeValue::List(dirs))
        }
        // Text intrinsics — pure/synchronous
        (IntrinsicValue::TextLength, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Int(s.chars().count() as i64))
        }
        (IntrinsicValue::TextByteLen, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Int(s.len() as i64))
        }
        (IntrinsicValue::TextSlice, [from, to, text]) => {
            let from = expect_intrinsic_i64(kernel, expr, value, 0, from)?;
            let to = expect_intrinsic_i64(kernel, expr, value, 1, to)?;
            let s = expect_intrinsic_text(kernel, expr, value, 2, text)?;
            let chars: Vec<char> = s.chars().collect();
            let from = (from.max(0) as usize).min(chars.len());
            let to = (to.max(0) as usize).min(chars.len()).max(from);
            let sliced: String = chars[from..to].iter().collect();
            Ok(RuntimeValue::Text(sliced.into()))
        }
        (IntrinsicValue::TextFind, [needle, haystack]) => {
            let needle = expect_intrinsic_text(kernel, expr, value, 0, needle)?;
            let haystack = expect_intrinsic_text(kernel, expr, value, 1, haystack)?;
            match haystack.find(needle.as_ref()) {
                Some(byte_idx) => {
                    let char_idx = haystack[..byte_idx].chars().count() as i64;
                    Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(
                        char_idx,
                    ))))
                }
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::TextContains, [needle, haystack]) => {
            let needle = expect_intrinsic_text(kernel, expr, value, 0, needle)?;
            let haystack = expect_intrinsic_text(kernel, expr, value, 1, haystack)?;
            Ok(RuntimeValue::Bool(haystack.contains(needle.as_ref())))
        }
        (IntrinsicValue::TextStartsWith, [prefix, text]) => {
            let prefix = expect_intrinsic_text(kernel, expr, value, 0, prefix)?;
            let text = expect_intrinsic_text(kernel, expr, value, 1, text)?;
            Ok(RuntimeValue::Bool(text.starts_with(prefix.as_ref())))
        }
        (IntrinsicValue::TextEndsWith, [suffix, text]) => {
            let suffix = expect_intrinsic_text(kernel, expr, value, 0, suffix)?;
            let text = expect_intrinsic_text(kernel, expr, value, 1, text)?;
            Ok(RuntimeValue::Bool(text.ends_with(suffix.as_ref())))
        }
        (IntrinsicValue::TextToUpper, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s.to_uppercase().into()))
        }
        (IntrinsicValue::TextToLower, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s.to_lowercase().into()))
        }
        (IntrinsicValue::TextTrim, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s.trim().into()))
        }
        (IntrinsicValue::TextTrimStart, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s.trim_start().into()))
        }
        (IntrinsicValue::TextTrimEnd, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s.trim_end().into()))
        }
        (IntrinsicValue::TextReplace, [needle, replacement, text]) => {
            let needle = expect_intrinsic_text(kernel, expr, value, 0, needle)?;
            let replacement = expect_intrinsic_text(kernel, expr, value, 1, replacement)?;
            let text = expect_intrinsic_text(kernel, expr, value, 2, text)?;
            let result = text.replacen(needle.as_ref(), replacement.as_ref(), 1);
            Ok(RuntimeValue::Text(result.into()))
        }
        (IntrinsicValue::TextReplaceAll, [needle, replacement, text]) => {
            let needle = expect_intrinsic_text(kernel, expr, value, 0, needle)?;
            let replacement = expect_intrinsic_text(kernel, expr, value, 1, replacement)?;
            let text = expect_intrinsic_text(kernel, expr, value, 2, text)?;
            Ok(RuntimeValue::Text(
                text.replace(needle.as_ref(), replacement.as_ref()).into(),
            ))
        }
        (IntrinsicValue::TextSplit, [separator, text]) => {
            let sep = expect_intrinsic_text(kernel, expr, value, 0, separator)?;
            let text = expect_intrinsic_text(kernel, expr, value, 1, text)?;
            let parts: Vec<RuntimeValue> = text
                .split(sep.as_ref())
                .map(|p| RuntimeValue::Text(p.into()))
                .collect();
            Ok(RuntimeValue::List(parts))
        }
        (IntrinsicValue::TextRepeat, [count, text]) => {
            let count = expect_intrinsic_i64(kernel, expr, value, 0, count)?.max(0) as usize;
            let text = expect_intrinsic_text(kernel, expr, value, 1, text)?;
            Ok(RuntimeValue::Text(text.repeat(count).into()))
        }
        (IntrinsicValue::TextFromInt, [n]) => {
            let n = expect_intrinsic_i64(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::Text(n.to_string().into()))
        }
        (IntrinsicValue::TextParseInt, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            match s.trim().parse::<i64>() {
                Ok(n) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(n)))),
                Err(_) => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::TextFromBool, [b]) => {
            let bv = match strip_signal(b.clone()) {
                RuntimeValue::Bool(v) => v,
                found => {
                    return Err(EvaluationError::InvalidIntrinsicArgument {
                        kernel,
                        expr,
                        value,
                        index: 0,
                        found,
                    });
                }
            };
            Ok(RuntimeValue::Text(if bv { "True" } else { "False" }.into()))
        }
        (IntrinsicValue::TextParseBool, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            match s.trim() {
                "True" => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(true)))),
                "False" => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(
                    false,
                )))),
                _ => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::TextConcat, [list]) => {
            let parts = match strip_signal(list.clone()) {
                RuntimeValue::List(v) => v,
                found => {
                    return Err(EvaluationError::InvalidIntrinsicArgument {
                        kernel,
                        expr,
                        value,
                        index: 0,
                        found,
                    });
                }
            };
            let mut result = String::new();
            for part in &parts {
                if let RuntimeValue::Text(t) = strip_signal(part.clone()) {
                    result.push_str(&t);
                }
            }
            Ok(RuntimeValue::Text(result.into()))
        }
        // Float transcendental intrinsics — pure/synchronous
        (IntrinsicValue::FloatSin, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.sin())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatSin,
                    reason: "sin result is not finite",
                })
        }
        (IntrinsicValue::FloatCos, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.cos())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatCos,
                    reason: "cos result is not finite",
                })
        }
        (IntrinsicValue::FloatTan, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.tan())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatTan,
                    reason: "tan result is not finite",
                })
        }
        (IntrinsicValue::FloatAsin, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            let result = f.asin();
            if result.is_finite() {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(result)
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatAsin,
                            reason: "asin result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatAcos, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            let result = f.acos();
            if result.is_finite() {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(result)
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatAcos,
                            reason: "acos result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatAtan, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.atan())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatAtan,
                    reason: "atan result is not finite",
                })
        }
        (IntrinsicValue::FloatAtan2, [y, x]) => {
            let y = expect_intrinsic_float(kernel, expr, value, 0, y)?;
            let x = expect_intrinsic_float(kernel, expr, value, 1, x)?;
            RuntimeFloat::new(y.atan2(x))
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatAtan2,
                    reason: "atan2 result is not finite",
                })
        }
        (IntrinsicValue::FloatExp, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.exp())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatExp,
                    reason: "exp result is not finite",
                })
        }
        (IntrinsicValue::FloatLog, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            if f > 0.0 {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(f.ln())
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatLog,
                            reason: "log result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatLog2, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            if f > 0.0 {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(f.log2())
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatLog2,
                            reason: "log2 result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatLog10, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            if f > 0.0 {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(f.log10())
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatLog10,
                            reason: "log10 result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatPow, [base, exp]) => {
            let base = expect_intrinsic_float(kernel, expr, value, 0, base)?;
            let exp = expect_intrinsic_float(kernel, expr, value, 1, exp)?;
            let result = base.powf(exp);
            if result.is_finite() {
                Ok(RuntimeValue::OptionSome(Box::new(
                    RuntimeFloat::new(result)
                        .map(RuntimeValue::Float)
                        .ok_or_else(|| EvaluationError::IntrinsicFailed {
                            kernel,
                            expr,
                            value: IntrinsicValue::FloatPow,
                            reason: "pow result is not finite",
                        })?,
                )))
            } else {
                Ok(RuntimeValue::OptionNone)
            }
        }
        (IntrinsicValue::FloatHypot, [a, b]) => {
            let a = expect_intrinsic_float(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_float(kernel, expr, value, 1, b)?;
            RuntimeFloat::new(a.hypot(b))
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatHypot,
                    reason: "hypot result is not finite",
                })
        }
        (IntrinsicValue::FloatTrunc, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.trunc())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatTrunc,
                    reason: "trunc result is not finite",
                })
        }
        (IntrinsicValue::FloatFrac, [n]) => {
            let f = expect_intrinsic_float(kernel, expr, value, 0, n)?;
            RuntimeFloat::new(f.fract())
                .map(RuntimeValue::Float)
                .ok_or_else(|| EvaluationError::IntrinsicFailed {
                    kernel,
                    expr,
                    value: IntrinsicValue::FloatFrac,
                    reason: "frac result is not finite",
                })
        }
        // Time intrinsics — Task-returning
        (IntrinsicValue::TimeNowMs, []) => Ok(RuntimeValue::Task(RuntimeTaskPlan::TimeNowMs)),
        (IntrinsicValue::TimeMonotonicMs, []) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::TimeMonotonicMs))
        }
        (IntrinsicValue::TimeFormat, [ms, pattern]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::TimeFormat {
                epoch_ms: expect_intrinsic_i64(kernel, expr, value, 0, ms)?,
                pattern: expect_intrinsic_text(kernel, expr, value, 1, pattern)?,
            }))
        }
        (IntrinsicValue::TimeParse, [text, pattern]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::TimeParse {
                text: expect_intrinsic_text(kernel, expr, value, 0, text)?,
                pattern: expect_intrinsic_text(kernel, expr, value, 1, pattern)?,
            }))
        }
        // Env intrinsics — Task-returning
        (IntrinsicValue::EnvGet, [name]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::EnvGet {
            name: expect_intrinsic_text(kernel, expr, value, 0, name)?,
        })),
        (IntrinsicValue::EnvList, [prefix]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::EnvList {
            prefix: expect_intrinsic_text(kernel, expr, value, 0, prefix)?,
        })),
        // Log intrinsics — Task-returning
        (IntrinsicValue::LogEmit, [level, message]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::LogEmit {
                level: expect_intrinsic_text(kernel, expr, value, 0, level)?,
                message: expect_intrinsic_text(kernel, expr, value, 1, message)?,
            }))
        }
        (IntrinsicValue::LogEmitContext, [level, message, context]) => {
            let level = expect_intrinsic_text(kernel, expr, value, 0, level)?;
            let message = expect_intrinsic_text(kernel, expr, value, 1, message)?;
            let context_list = match strip_signal(context.clone()) {
                RuntimeValue::List(v) => v,
                found => {
                    return Err(EvaluationError::InvalidIntrinsicArgument {
                        kernel,
                        expr,
                        value,
                        index: 2,
                        found,
                    });
                }
            };
            let mut pairs: Vec<(Box<str>, Box<str>)> = Vec::with_capacity(context_list.len());
            for entry in &context_list {
                match strip_signal(entry.clone()) {
                    RuntimeValue::Tuple(elements) if elements.len() == 2 => {
                        let k = match strip_signal(elements[0].clone()) {
                            RuntimeValue::Text(t) => t,
                            found => {
                                return Err(EvaluationError::InvalidIntrinsicArgument {
                                    kernel,
                                    expr,
                                    value,
                                    index: 2,
                                    found,
                                });
                            }
                        };
                        let v = match strip_signal(elements[1].clone()) {
                            RuntimeValue::Text(t) => t,
                            found => {
                                return Err(EvaluationError::InvalidIntrinsicArgument {
                                    kernel,
                                    expr,
                                    value,
                                    index: 2,
                                    found,
                                });
                            }
                        };
                        pairs.push((k, v));
                    }
                    found => {
                        return Err(EvaluationError::InvalidIntrinsicArgument {
                            kernel,
                            expr,
                            value,
                            index: 2,
                            found,
                        });
                    }
                }
            }
            Ok(RuntimeValue::Task(RuntimeTaskPlan::LogEmitContext {
                level,
                message,
                context: pairs.into_boxed_slice(),
            }))
        }
        // Random float — Task-returning
        (IntrinsicValue::RandomFloat, []) => Ok(RuntimeValue::Task(RuntimeTaskPlan::RandomFloat)),
        // I18n intrinsics — pure/synchronous
        (IntrinsicValue::I18nTranslate, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            Ok(RuntimeValue::Text(s))
        }
        (IntrinsicValue::I18nTranslatePlural, [singular, plural, count]) => {
            let singular = expect_intrinsic_text(kernel, expr, value, 0, singular)?;
            let plural = expect_intrinsic_text(kernel, expr, value, 1, plural)?;
            let count = expect_intrinsic_i64(kernel, expr, value, 2, count)?;
            Ok(RuntimeValue::Text(if count == 1 {
                singular
            } else {
                plural
            }))
        }
        // Regex intrinsics — Task-returning
        (IntrinsicValue::RegexIsMatch, [pattern, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexIsMatch {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                text: expect_intrinsic_text(kernel, expr, value, 1, text)?,
            }))
        }
        (IntrinsicValue::RegexFind, [pattern, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexFind {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                text: expect_intrinsic_text(kernel, expr, value, 1, text)?,
            }))
        }
        (IntrinsicValue::RegexFindText, [pattern, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexFindText {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                text: expect_intrinsic_text(kernel, expr, value, 1, text)?,
            }))
        }
        (IntrinsicValue::RegexFindAll, [pattern, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexFindAll {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                text: expect_intrinsic_text(kernel, expr, value, 1, text)?,
            }))
        }
        (IntrinsicValue::RegexReplace, [pattern, replacement, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexReplace {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                replacement: expect_intrinsic_text(kernel, expr, value, 1, replacement)?,
                text: expect_intrinsic_text(kernel, expr, value, 2, text)?,
            }))
        }
        (IntrinsicValue::RegexReplaceAll, [pattern, replacement, text]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::RegexReplaceAll {
                pattern: expect_intrinsic_text(kernel, expr, value, 0, pattern)?,
                replacement: expect_intrinsic_text(kernel, expr, value, 1, replacement)?,
                text: expect_intrinsic_text(kernel, expr, value, 2, text)?,
            }))
        }
        (IntrinsicValue::HttpGet, [url]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpGet {
            url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
        })),
        (IntrinsicValue::HttpGetBytes, [url]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpGetBytes {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
            }))
        }
        (IntrinsicValue::HttpGetStatus, [url]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpGetStatus {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
            }))
        }
        (IntrinsicValue::HttpDelete, [url]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpDelete {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
            }))
        }
        (IntrinsicValue::HttpHead, [url]) => Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpHead {
            url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
        })),
        (IntrinsicValue::HttpPostJson, [url, body]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpPostJson {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
                body: expect_intrinsic_text(kernel, expr, value, 1, body)?,
            }))
        }
        (IntrinsicValue::HttpPost, [url, content_type, body]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpPost {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
                content_type: expect_intrinsic_text(kernel, expr, value, 1, content_type)?,
                body: expect_intrinsic_text(kernel, expr, value, 2, body)?,
            }))
        }
        (IntrinsicValue::HttpPut, [url, content_type, body]) => {
            Ok(RuntimeValue::Task(RuntimeTaskPlan::HttpPut {
                url: expect_intrinsic_text(kernel, expr, value, 0, url)?,
                content_type: expect_intrinsic_text(kernel, expr, value, 1, content_type)?,
                body: expect_intrinsic_text(kernel, expr, value, 2, body)?,
            }))
        }
        // BigInt intrinsics — pure, no I/O
        (IntrinsicValue::BigIntFromInt, [n]) => {
            let n = expect_intrinsic_i64(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::BigInt(RuntimeBigInt::from_i64(n)))
        }
        (IntrinsicValue::BigIntFromText, [text]) => {
            let s = expect_intrinsic_text(kernel, expr, value, 0, text)?;
            match RuntimeBigInt::from_decimal_str(&s) {
                Some(b) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::BigInt(b)))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::BigIntToInt, [n]) => {
            let b = expect_intrinsic_bigint(kernel, expr, value, 0, n)?;
            match b.to_i64() {
                Some(n) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(n)))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::BigIntToText, [n]) => {
            let b = expect_intrinsic_bigint(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::Text(b.to_decimal_str()))
        }
        (IntrinsicValue::BigIntAdd, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::BigInt(a.bigint_add(&b)))
        }
        (IntrinsicValue::BigIntSub, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::BigInt(a.bigint_sub(&b)))
        }
        (IntrinsicValue::BigIntMul, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::BigInt(a.bigint_mul(&b)))
        }
        (IntrinsicValue::BigIntDiv, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            match a.bigint_div(&b) {
                Some(r) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::BigInt(r)))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::BigIntMod, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            match a.bigint_rem(&b) {
                Some(r) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::BigInt(r)))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        (IntrinsicValue::BigIntPow, [base, exp]) => {
            let base = expect_intrinsic_bigint(kernel, expr, value, 0, base)?;
            let exp = expect_intrinsic_i64(kernel, expr, value, 1, exp)?.max(0) as u32;
            Ok(RuntimeValue::BigInt(base.bigint_pow(exp)))
        }
        (IntrinsicValue::BigIntNeg, [n]) => {
            let b = expect_intrinsic_bigint(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::BigInt(b.bigint_neg()))
        }
        (IntrinsicValue::BigIntAbs, [n]) => {
            let b = expect_intrinsic_bigint(kernel, expr, value, 0, n)?;
            Ok(RuntimeValue::BigInt(b.bigint_abs()))
        }
        (IntrinsicValue::BigIntCmp, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::Int(match a.cmp(&b) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        }
        (IntrinsicValue::BigIntEq, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::Bool(a == b))
        }
        (IntrinsicValue::BigIntGt, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::Bool(a > b))
        }
        (IntrinsicValue::BigIntLt, [a, b]) => {
            let a = expect_intrinsic_bigint(kernel, expr, value, 0, a)?;
            let b = expect_intrinsic_bigint(kernel, expr, value, 1, b)?;
            Ok(RuntimeValue::Bool(a < b))
        }
        _ => unreachable!("intrinsic arity should be enforced before evaluation"),
    }
}

fn runtime_custom_capability_command_plan(
    arguments: Vec<RuntimeValue>,
    spec: &aivi_hir::CustomCapabilityCommandSpec,
) -> RuntimeCustomCapabilityCommandPlan {
    let mut arguments = arguments.into_iter().map(strip_signal);
    let provider_arguments = spec
        .provider_arguments
        .iter()
        .map(|name| RuntimeNamedValue {
            name: name.clone(),
            value: arguments
                .next()
                .expect("custom capability command provider arguments should stay aligned"),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let options = spec
        .options
        .iter()
        .map(|name| RuntimeNamedValue {
            name: name.clone(),
            value: arguments
                .next()
                .expect("custom capability command options should stay aligned"),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let command_arguments = spec
        .arguments
        .iter()
        .map(|name| RuntimeNamedValue {
            name: name.clone(),
            value: arguments
                .next()
                .expect("custom capability command member arguments should stay aligned"),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    RuntimeCustomCapabilityCommandPlan {
        provider_key: spec.provider_key.clone(),
        command: spec.command.clone(),
        provider_arguments,
        options,
        arguments: command_arguments,
    }
}

fn builtin_class_member_arity(intrinsic: BuiltinClassMemberIntrinsic) -> usize {
    match intrinsic {
        BuiltinClassMemberIntrinsic::Empty(_) => 0,
        BuiltinClassMemberIntrinsic::Pure(_) | BuiltinClassMemberIntrinsic::Join(_) => 1,
        BuiltinClassMemberIntrinsic::Bimap(_) | BuiltinClassMemberIntrinsic::Reduce(_) => 3,
        BuiltinClassMemberIntrinsic::StructuralEq
        | BuiltinClassMemberIntrinsic::Compare { .. }
        | BuiltinClassMemberIntrinsic::Append(_)
        | BuiltinClassMemberIntrinsic::Map(_)
        | BuiltinClassMemberIntrinsic::Apply(_)
        | BuiltinClassMemberIntrinsic::Traverse { .. }
        | BuiltinClassMemberIntrinsic::FilterMap(_)
        | BuiltinClassMemberIntrinsic::Chain(_) => 2,
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
        BuiltinApplicativeCarrier::Task => RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(payload),
        }),
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
        BuiltinApplicativeCarrier::Task => match strip_signal(mapped) {
            RuntimeValue::Task(plan) => Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::OptionSome(Box::new(RuntimeValue::Task(plan)))),
            })),
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
        BuiltinApplicativeCarrier::Task => match strip_signal(mapped) {
            RuntimeValue::Task(plan) => Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Task(plan)))),
            })),
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
        BuiltinApplicativeCarrier::Task => match strip_signal(mapped) {
            RuntimeValue::Task(plan) => Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::ValidationValid(Box::new(RuntimeValue::Task(
                    plan,
                )))),
            })),
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
        BuiltinApplicativeCarrier::Task => {
            let mut collected = Vec::with_capacity(mapped.len());
            for value in mapped {
                match strip_signal(value) {
                    RuntimeValue::Task(plan) => collected.push(RuntimeValue::Task(plan)),
                    _ => {
                        return Err(
                            "traverse expected the mapped value to stay in the target applicative",
                        );
                    }
                }
            }
            Ok(RuntimeValue::Task(RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::List(collected)),
            }))
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

fn expect_intrinsic_text(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<Box<str>, EvaluationError> {
    match strip_signal(argument.clone()) {
        RuntimeValue::Text(found) => Ok(found),
        found => Err(EvaluationError::InvalidIntrinsicArgument {
            kernel,
            expr,
            value,
            index,
            found: found.clone(),
        }),
    }
}

fn expect_intrinsic_bytes(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<Box<[u8]>, EvaluationError> {
    match strip_signal(argument.clone()) {
        RuntimeValue::Bytes(found) => Ok(found),
        found => Err(EvaluationError::InvalidIntrinsicArgument {
            kernel,
            expr,
            value,
            index,
            found: found.clone(),
        }),
    }
}

fn expect_intrinsic_float(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<f64, EvaluationError> {
    match strip_signal(argument.clone()) {
        RuntimeValue::Float(found) => Ok(found.to_f64()),
        found => Err(EvaluationError::InvalidIntrinsicArgument {
            kernel,
            expr,
            value,
            index,
            found: found.clone(),
        }),
    }
}

fn expect_intrinsic_bigint(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<RuntimeBigInt, EvaluationError> {
    match strip_signal(argument.clone()) {
        RuntimeValue::BigInt(found) => Ok(found),
        found => Err(EvaluationError::InvalidIntrinsicArgument {
            kernel,
            expr,
            value,
            index,
            found,
        }),
    }
}

fn invalid_intrinsic_argument(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    found: RuntimeValue,
) -> EvaluationError {
    EvaluationError::InvalidIntrinsicArgument {
        kernel,
        expr,
        value,
        index,
        found,
    }
}

fn runtime_record_field(label: &str, value: RuntimeValue) -> RuntimeRecordField {
    RuntimeRecordField {
        label: label.into(),
        value,
    }
}

fn runtime_db_param(
    kind: &'static str,
    payload_field: &'static str,
    payload: RuntimeValue,
) -> RuntimeValue {
    let payload_slot = |field| {
        if field == payload_field {
            RuntimeValue::OptionSome(Box::new(payload.clone()))
        } else {
            RuntimeValue::OptionNone
        }
    };
    RuntimeValue::Record(vec![
        runtime_record_field("kind", RuntimeValue::Text(kind.into())),
        runtime_record_field("bool", payload_slot("bool")),
        runtime_record_field("int", payload_slot("int")),
        runtime_record_field("float", payload_slot("float")),
        runtime_record_field("decimal", payload_slot("decimal")),
        runtime_record_field("bigInt", payload_slot("bigInt")),
        runtime_record_field("text", payload_slot("text")),
        runtime_record_field("bytes", payload_slot("bytes")),
    ])
}

fn runtime_db_statement(sql: Box<str>, arguments: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Record(vec![
        runtime_record_field("sql", RuntimeValue::Text(sql)),
        runtime_record_field("arguments", RuntimeValue::List(arguments)),
    ])
}

fn record_field<'a>(fields: &'a [RuntimeRecordField], label: &str) -> Option<&'a RuntimeValue> {
    fields
        .iter()
        .find(|field| field.label.as_ref() == label)
        .map(|field| &field.value)
}

fn expect_intrinsic_text_list(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<Vec<Box<str>>, EvaluationError> {
    let found = strip_signal(argument.clone());
    let RuntimeValue::List(values) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    values
        .iter()
        .map(|entry| match strip_signal(entry.clone()) {
            RuntimeValue::Text(text) => Ok(text),
            found => Err(invalid_intrinsic_argument(
                kernel, expr, value, index, found,
            )),
        })
        .collect()
}

fn expect_intrinsic_db_connection(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<RuntimeDbConnection, EvaluationError> {
    let found = strip_signal(argument.clone());
    let RuntimeValue::Record(fields) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    let Some(database) = record_field(fields, "database") else {
        return Err(invalid_intrinsic_argument(
            kernel,
            expr,
            value,
            index,
            found.clone(),
        ));
    };
    match strip_signal(database.clone()) {
        RuntimeValue::Text(database) => Ok(RuntimeDbConnection { database }),
        found => Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        )),
    }
}

fn expect_intrinsic_db_statement_list(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<Vec<RuntimeDbStatement>, EvaluationError> {
    let found = strip_signal(argument.clone());
    let RuntimeValue::List(values) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    values
        .iter()
        .map(|statement| expect_intrinsic_db_statement(kernel, expr, value, index, statement))
        .collect()
}

fn expect_intrinsic_db_statement(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<RuntimeDbStatement, EvaluationError> {
    let found = strip_signal(argument.clone());
    let RuntimeValue::Record(fields) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    let Some(sql) = record_field(fields, "sql") else {
        return Err(invalid_intrinsic_argument(
            kernel,
            expr,
            value,
            index,
            found.clone(),
        ));
    };
    let Some(arguments) = record_field(fields, "arguments") else {
        return Err(invalid_intrinsic_argument(
            kernel,
            expr,
            value,
            index,
            found.clone(),
        ));
    };
    let sql = match strip_signal(sql.clone()) {
        RuntimeValue::Text(sql) => sql,
        found => {
            return Err(invalid_intrinsic_argument(
                kernel, expr, value, index, found,
            ));
        }
    };
    let arguments = expect_intrinsic_db_statement_arguments(kernel, expr, value, index, arguments)?;
    Ok(RuntimeDbStatement { sql, arguments })
}

fn expect_intrinsic_db_statement_arguments(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<Vec<RuntimeValue>, EvaluationError> {
    let found = strip_signal(argument.clone());
    let RuntimeValue::List(values) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    values
        .iter()
        .map(|argument| expect_intrinsic_db_param(kernel, expr, value, index, argument))
        .collect()
}

fn expect_intrinsic_db_param(
    kernel: KernelId,
    expr: KernelExprId,
    value: IntrinsicValue,
    index: usize,
    argument: &RuntimeValue,
) -> Result<RuntimeValue, EvaluationError> {
    const PAYLOAD_FIELDS: [&str; 7] =
        ["bool", "int", "float", "decimal", "bigInt", "text", "bytes"];

    let found = strip_signal(argument.clone());
    let RuntimeValue::Record(fields) = &found else {
        return Err(invalid_intrinsic_argument(
            kernel, expr, value, index, found,
        ));
    };
    let Some(kind) = record_field(fields, "kind") else {
        return Err(invalid_intrinsic_argument(
            kernel,
            expr,
            value,
            index,
            found.clone(),
        ));
    };
    let kind = match strip_signal(kind.clone()) {
        RuntimeValue::Text(kind) => kind,
        found => {
            return Err(invalid_intrinsic_argument(
                kernel, expr, value, index, found,
            ));
        }
    };
    if !PAYLOAD_FIELDS.contains(&kind.as_ref()) {
        return Err(invalid_intrinsic_argument(
            kernel,
            expr,
            value,
            index,
            found.clone(),
        ));
    }
    for field in PAYLOAD_FIELDS {
        let Some(value_field) = record_field(fields, field) else {
            return Err(invalid_intrinsic_argument(
                kernel,
                expr,
                value,
                index,
                found.clone(),
            ));
        };
        let runtime_value = strip_signal(value_field.clone());
        if field == kind.as_ref() {
            let RuntimeValue::OptionSome(payload) = runtime_value else {
                return Err(invalid_intrinsic_argument(
                    kernel,
                    expr,
                    value,
                    index,
                    found.clone(),
                ));
            };
            return Ok(*payload);
        }
        if !matches!(runtime_value, RuntimeValue::OptionNone) {
            return Err(invalid_intrinsic_argument(
                kernel,
                expr,
                value,
                index,
                found.clone(),
            ));
        }
    }
    Err(invalid_intrinsic_argument(
        kernel, expr, value, index, found,
    ))
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

fn callable_signature(program: &Program, layout: LayoutId) -> (Vec<LayoutId>, LayoutId) {
    let mut parameters = Vec::new();
    let mut result = layout;
    loop {
        let Some(layout) = program.layouts().get(result) else {
            return (parameters, result);
        };
        let LayoutKind::Arrow {
            parameter,
            result: next_result,
        } = &layout.kind
        else {
            return (parameters, result);
        };
        parameters.push(*parameter);
        result = *next_result;
    }
}

fn is_named_domain_layout(program: &Program, layout: LayoutId) -> bool {
    matches!(
        program.layouts().get(layout).map(|layout| &layout.kind),
        Some(LayoutKind::Domain { .. })
    )
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

fn domain_member_carrier_value(value: RuntimeValue) -> RuntimeValue {
    match strip_signal(value) {
        RuntimeValue::SuffixedInteger { raw, suffix } => raw
            .parse::<i64>()
            .map(RuntimeValue::Int)
            .unwrap_or(RuntimeValue::SuffixedInteger { raw, suffix }),
        other => other,
    }
}

fn coerce_domain_numeric_value(value: RuntimeValue) -> Option<RuntimeValue> {
    match strip_signal(value) {
        RuntimeValue::SuffixedInteger { raw, .. } => raw.parse::<i64>().ok().map(RuntimeValue::Int),
        other => Some(other),
    }
}

fn shared_suffixed_integer_suffix(
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Option<Option<Box<str>>> {
    match (left, right) {
        (
            RuntimeValue::SuffixedInteger {
                suffix: left_suffix,
                ..
            },
            RuntimeValue::SuffixedInteger {
                suffix: right_suffix,
                ..
            },
        ) if left_suffix == right_suffix => Some(Some(left_suffix.clone())),
        (RuntimeValue::SuffixedInteger { .. }, RuntimeValue::SuffixedInteger { .. }) => None,
        _ => Some(None),
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
        (LayoutKind::List { element }, RuntimeValue::List(elements))
        | (LayoutKind::Set { element }, RuntimeValue::Set(elements)) => elements
            .iter()
            .all(|value| value_matches_layout(program, value, *element)),
        (LayoutKind::Map { key, value }, RuntimeValue::Map(entries)) => {
            entries.iter().all(|(k, v)| {
                value_matches_layout(program, k, *key) && value_matches_layout(program, v, *value)
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
        (LayoutKind::AnonymousDomain { .. }, RuntimeValue::SuffixedInteger { .. }) => true,
        (LayoutKind::Domain { .. }, RuntimeValue::Signal(_)) => false,
        // Named-domain layouts erase their carrier shape in backend IR. Runtime evaluation relies
        // on earlier typed lowering to keep those carrier values sound and only preserves the
        // outer signal/non-signal distinction here.
        (LayoutKind::Domain { .. }, _) => true,
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
    use std::collections::BTreeSet;

    use aivi_hir::{ItemId as HirItemId, SumConstructorHandle};

    use super::{
        DetachedRuntimeValue, RuntimeDbCommitPlan, RuntimeDbConnection, RuntimeDbQueryPlan,
        RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeMap, RuntimeMapEntry, RuntimeRecordField,
        RuntimeSumValue, RuntimeValue, append_validation_errors, structural_eq,
    };
    use crate::{KernelExprId, KernelId};

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
                value: RuntimeValue::Map(RuntimeMap::from_entries(vec![RuntimeMapEntry {
                    key: RuntimeValue::Text("attempts".into()),
                    value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
                }])),
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
    fn display_preserves_runtime_map_entry_order() {
        let value = RuntimeValue::Map(RuntimeMap::from_entries(vec![
            RuntimeMapEntry {
                key: RuntimeValue::Text("zeta".into()),
                value: RuntimeValue::Int(1),
            },
            RuntimeMapEntry {
                key: RuntimeValue::Text("alpha".into()),
                value: RuntimeValue::Int(2),
            },
        ]));

        assert_eq!(value.display_text(), "{zeta: 1, alpha: 2}");
        assert_eq!(format!("{value}"), "{zeta: 1, alpha: 2}");
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
    fn db_task_plan_display_formats_query_work() {
        let plan = RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
            connection: RuntimeDbConnection {
                database: "/var/lib/app.sqlite".into(),
            },
            statement: RuntimeDbStatement {
                sql: "select * from users where id = ?".into(),
                arguments: vec![RuntimeValue::Int(7)],
            },
        });

        assert_eq!(
            format!("{plan}"),
            "db.query(db.connection(/var/lib/app.sqlite), sql(select * from users where id = ?; args: [7]))"
        );
        assert_eq!(
            format!("{plan:?}"),
            r#"Query(RuntimeDbQueryPlan { connection: RuntimeDbConnection { database: "/var/lib/app.sqlite" }, statement: RuntimeDbStatement { sql: "select * from users where id = ?", arguments: [Int(7)] } })"#
        );
    }

    #[test]
    fn db_task_plan_display_formats_commit_work_deterministically() {
        let plan = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: RuntimeDbConnection {
                database: "/var/lib/app.sqlite".into(),
            },
            statements: vec![
                RuntimeDbStatement {
                    sql: "insert into users(id, name) values (?, ?)".into(),
                    arguments: vec![RuntimeValue::Int(7), RuntimeValue::Text("Ada".into())],
                },
                RuntimeDbStatement {
                    sql: "insert into audit_log(message) values (?)".into(),
                    arguments: vec![RuntimeValue::Text("created user".into())],
                },
            ],
            changed_tables: ["users", "audit_log"].into_iter().map(Into::into).collect(),
        });

        assert_eq!(
            format!("{plan}"),
            "db.commit(db.connection(/var/lib/app.sqlite), [sql(insert into users(id, name) values (?, ?); args: [7, Ada]), sql(insert into audit_log(message) values (?); args: [created user])]; changes: [audit_log, users])"
        );
    }

    #[test]
    fn db_commit_plan_equality_normalizes_changed_table_order() {
        let left = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: RuntimeDbConnection {
                database: "/var/lib/app.sqlite".into(),
            },
            statements: vec![RuntimeDbStatement {
                sql: "update users set active = ? where id = ?".into(),
                arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
            }],
            changed_tables: ["users", "audit_log"].into_iter().map(Into::into).collect(),
        });
        let right = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: RuntimeDbConnection {
                database: "/var/lib/app.sqlite".into(),
            },
            statements: vec![RuntimeDbStatement {
                sql: "update users set active = ? where id = ?".into(),
                arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
            }],
            changed_tables: ["audit_log", "users"].into_iter().map(Into::into).collect(),
        });

        assert_eq!(left, right);
    }

    #[test]
    fn db_commit_plan_equality_tracks_invalidation_and_statement_payload() {
        let base_connection = RuntimeDbConnection {
            database: "/var/lib/app.sqlite".into(),
        };
        let base_statement = RuntimeDbStatement {
            sql: "update users set active = ? where id = ?".into(),
            arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
        };
        let left = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: base_connection.clone(),
            statements: vec![base_statement.clone()],
            changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
        });
        let different_tables = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: base_connection.clone(),
            statements: vec![base_statement.clone()],
            changed_tables: BTreeSet::from(["users".into()]),
        });
        let different_statement = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: base_connection,
            statements: vec![RuntimeDbStatement {
                sql: "update users set active = ? where id = ?".into(),
                arguments: vec![RuntimeValue::Bool(false), RuntimeValue::Int(7)],
            }],
            changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
        });

        assert_ne!(left, different_tables);
        assert_ne!(left, different_statement);
    }

    #[test]
    fn structural_equality_handles_bytes_maps_and_sets() {
        let kernel = KernelId::from_raw(0);
        let expr = KernelExprId::from_raw(0);

        assert!(
            structural_eq(
                kernel,
                expr,
                &RuntimeValue::Bytes([1, 2, 3].into()),
                &RuntimeValue::Bytes([1, 2, 3].into()),
            )
            .expect("bytes should compare structurally")
        );

        let left_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
            RuntimeMapEntry {
                key: RuntimeValue::Text("left".into()),
                value: RuntimeValue::Int(1),
            },
            RuntimeMapEntry {
                key: RuntimeValue::Text("right".into()),
                value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
            },
        ]));
        let right_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
            RuntimeMapEntry {
                key: RuntimeValue::Text("right".into()),
                value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
            },
            RuntimeMapEntry {
                key: RuntimeValue::Text("left".into()),
                value: RuntimeValue::Int(1),
            },
        ]));
        assert!(
            structural_eq(kernel, expr, &left_map, &right_map)
                .expect("maps should compare structurally regardless of insertion order")
        );

        let left_set = RuntimeValue::Set(vec![RuntimeValue::Int(1), RuntimeValue::Int(2)]);
        let right_set = RuntimeValue::Set(vec![RuntimeValue::Int(2), RuntimeValue::Int(1)]);
        assert!(
            structural_eq(kernel, expr, &left_set, &right_set)
                .expect("sets should compare structurally regardless of insertion order")
        );
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

    #[test]
    fn structural_equality_matches_bytes_maps_and_sets() {
        let kernel = KernelId::from_raw(0);
        let expr = KernelExprId::from_raw(0);

        assert!(
            structural_eq(
                kernel,
                expr,
                &RuntimeValue::Bytes(Box::from(*b"abc")),
                &RuntimeValue::Bytes(Box::from(*b"abc")),
            )
            .expect("bytes equality should be supported")
        );

        let left_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
            RuntimeMapEntry {
                key: RuntimeValue::Text("first".into()),
                value: RuntimeValue::Int(1),
            },
            RuntimeMapEntry {
                key: RuntimeValue::Text("second".into()),
                value: RuntimeValue::List(vec![
                    RuntimeValue::Bool(true),
                    RuntimeValue::Bool(false),
                ]),
            },
        ]));
        let right_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
            RuntimeMapEntry {
                key: RuntimeValue::Text("second".into()),
                value: RuntimeValue::List(vec![
                    RuntimeValue::Bool(true),
                    RuntimeValue::Bool(false),
                ]),
            },
            RuntimeMapEntry {
                key: RuntimeValue::Text("first".into()),
                value: RuntimeValue::Int(1),
            },
        ]));
        assert!(
            structural_eq(kernel, expr, &left_map, &right_map)
                .expect("map equality should be order-independent")
        );

        let left_set = RuntimeValue::Set(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Text("two".into()),
            RuntimeValue::Bool(true),
        ]);
        let right_set = RuntimeValue::Set(vec![
            RuntimeValue::Bool(true),
            RuntimeValue::Int(1),
            RuntimeValue::Text("two".into()),
        ]);
        assert!(
            structural_eq(kernel, expr, &left_set, &right_set)
                .expect("set equality should be order-independent")
        );
    }
}
