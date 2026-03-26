use std::fmt;
use std::sync::OnceLock;

use crate::{Kind, KindExprId, KindStore, PrimitiveType};

/// Closed compiler-known built-in `@source` provider variants from RFC §14.1.2.
///
/// # Known limitation — closed enum
///
/// This is a closed, exhaustive enum. Every built-in provider the compiler recognises must be
/// listed here. There is no runtime extension point: adding a new built-in provider requires
/// modifying this enum, its `ALL` constant, the `parse` and `key` methods, and the `contract`
/// method. It is **not** extensible without a code change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum BuiltinSourceProvider {
    HttpGet,
    HttpPost,
    TimerEvery,
    TimerAfter,
    FsWatch,
    FsRead,
    SocketConnect,
    MailboxSubscribe,
    ProcessSpawn,
    ProcessArgs,
    ProcessCwd,
    EnvGet,
    StdioRead,
    PathHome,
    PathConfigHome,
    PathDataHome,
    PathCacheHome,
    PathTempDir,
    WindowKeyDown,
}

impl BuiltinSourceProvider {
    pub const ALL: [Self; 19] = [
        Self::HttpGet,
        Self::HttpPost,
        Self::TimerEvery,
        Self::TimerAfter,
        Self::FsWatch,
        Self::FsRead,
        Self::SocketConnect,
        Self::MailboxSubscribe,
        Self::ProcessSpawn,
        Self::ProcessArgs,
        Self::ProcessCwd,
        Self::EnvGet,
        Self::StdioRead,
        Self::PathHome,
        Self::PathConfigHome,
        Self::PathDataHome,
        Self::PathCacheHome,
        Self::PathTempDir,
        Self::WindowKeyDown,
    ];

    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "http.get" => Some(Self::HttpGet),
            "http.post" => Some(Self::HttpPost),
            "timer.every" => Some(Self::TimerEvery),
            "timer.after" => Some(Self::TimerAfter),
            "fs.watch" => Some(Self::FsWatch),
            "fs.read" => Some(Self::FsRead),
            "socket.connect" => Some(Self::SocketConnect),
            "mailbox.subscribe" => Some(Self::MailboxSubscribe),
            "process.spawn" => Some(Self::ProcessSpawn),
            "process.args" => Some(Self::ProcessArgs),
            "process.cwd" => Some(Self::ProcessCwd),
            "env.get" => Some(Self::EnvGet),
            "stdio.read" => Some(Self::StdioRead),
            "path.home" => Some(Self::PathHome),
            "path.configHome" => Some(Self::PathConfigHome),
            "path.dataHome" => Some(Self::PathDataHome),
            "path.cacheHome" => Some(Self::PathCacheHome),
            "path.tempDir" => Some(Self::PathTempDir),
            "window.keyDown" => Some(Self::WindowKeyDown),
            _ => None,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::HttpGet => "http.get",
            Self::HttpPost => "http.post",
            Self::TimerEvery => "timer.every",
            Self::TimerAfter => "timer.after",
            Self::FsWatch => "fs.watch",
            Self::FsRead => "fs.read",
            Self::SocketConnect => "socket.connect",
            Self::MailboxSubscribe => "mailbox.subscribe",
            Self::ProcessSpawn => "process.spawn",
            Self::ProcessArgs => "process.args",
            Self::ProcessCwd => "process.cwd",
            Self::EnvGet => "env.get",
            Self::StdioRead => "stdio.read",
            Self::PathHome => "path.home",
            Self::PathConfigHome => "path.configHome",
            Self::PathDataHome => "path.dataHome",
            Self::PathCacheHome => "path.cacheHome",
            Self::PathTempDir => "path.tempDir",
            Self::WindowKeyDown => "window.keyDown",
        }
    }

    pub fn contract(self) -> SourceContract {
        match self {
            Self::HttpGet => {
                SourceContract::new(self, http_options(), HTTP_RECURRENCE, HTTP_LIFECYCLE)
            }
            Self::HttpPost => {
                SourceContract::new(self, http_options(), HTTP_RECURRENCE, HTTP_LIFECYCLE)
            }
            Self::TimerEvery => {
                SourceContract::new(self, timer_options(), TIMER_RECURRENCE, TIMER_LIFECYCLE)
            }
            Self::TimerAfter => {
                SourceContract::new(self, timer_options(), TIMER_RECURRENCE, TIMER_LIFECYCLE)
            }
            Self::FsWatch => SourceContract::new(
                self,
                fs_watch_options(),
                FS_WATCH_RECURRENCE,
                STREAM_LIFECYCLE,
            ),
            Self::FsRead => {
                SourceContract::new(self, fs_read_options(), FS_READ_RECURRENCE, HTTP_LIFECYCLE)
            }
            Self::SocketConnect => {
                SourceContract::new(self, socket_options(), SOCKET_RECURRENCE, STREAM_LIFECYCLE)
            }
            Self::MailboxSubscribe => {
                SourceContract::new(self, socket_options(), MAILBOX_RECURRENCE, STREAM_LIFECYCLE)
            }
            Self::ProcessSpawn => SourceContract::new(
                self,
                process_options(),
                PROCESS_RECURRENCE,
                STREAM_LIFECYCLE,
            ),
            Self::ProcessArgs
            | Self::ProcessCwd
            | Self::EnvGet
            | Self::StdioRead
            | Self::PathHome
            | Self::PathConfigHome
            | Self::PathDataHome
            | Self::PathCacheHome
            | Self::PathTempDir => {
                SourceContract::new(self, &NO_OPTIONS, STATIC_RECURRENCE, STATIC_LIFECYCLE)
            }
            Self::WindowKeyDown => {
                SourceContract::new(self, &WINDOW_OPTIONS, WINDOW_RECURRENCE, STREAM_LIFECYCLE)
            }
        }
    }
}

/// Provider-keyed built-in source contract metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceContract {
    provider: BuiltinSourceProvider,
    options: &'static [SourceOptionContract],
    recurrence: SourceRecurrenceContract,
    lifecycle: SourceLifecycleContract,
}

impl SourceContract {
    pub fn new(
        provider: BuiltinSourceProvider,
        options: &'static [SourceOptionContract],
        recurrence: SourceRecurrenceContract,
        lifecycle: SourceLifecycleContract,
    ) -> Self {
        Self {
            provider,
            options,
            recurrence,
            lifecycle,
        }
    }

    pub const fn provider(self) -> BuiltinSourceProvider {
        self.provider
    }

    pub fn options(self) -> &'static [SourceOptionContract] {
        self.options
    }

    pub fn option(self, name: &str) -> Option<&'static SourceOptionContract> {
        self.options.iter().find(|option| option.name() == name)
    }

    pub const fn recurrence(self) -> SourceRecurrenceContract {
        self.recurrence
    }

    pub const fn lifecycle(self) -> SourceLifecycleContract {
        self.lifecycle
    }

    pub const fn intrinsic_wakeup(self) -> Option<SourceContractIntrinsicWakeup> {
        self.recurrence.intrinsic_wakeup()
    }

    pub fn wakeup_option(self, name: &str) -> Option<&'static SourceOptionWakeupContract> {
        self.recurrence.option(name)
    }
}

/// One legal option field for a built-in source provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOptionContract {
    name: &'static str,
    ty: SourceContractType,
}

impl SourceOptionContract {
    pub const fn new(name: &'static str, ty: SourceContractType) -> Self {
        Self { name, ty }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub fn ty(&self) -> SourceContractType {
        self.ty.clone()
    }
}

/// Built-in recurrence semantics attached to a source provider contract.
///
/// This stays in the typed source-contract layer so later HIR validation does not hard-code
/// provider wakeup behavior separately from option legality. Future custom-provider declarations
/// can populate the same shape once the language has a real declaration surface for them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceRecurrenceContract {
    intrinsic_wakeup: Option<SourceContractIntrinsicWakeup>,
    option_wakeups: &'static [SourceOptionWakeupContract],
}

impl SourceRecurrenceContract {
    pub const fn new(
        intrinsic_wakeup: Option<SourceContractIntrinsicWakeup>,
        option_wakeups: &'static [SourceOptionWakeupContract],
    ) -> Self {
        Self {
            intrinsic_wakeup,
            option_wakeups,
        }
    }

    pub const fn intrinsic_wakeup(self) -> Option<SourceContractIntrinsicWakeup> {
        self.intrinsic_wakeup
    }

    pub fn option(self, name: &str) -> Option<&'static SourceOptionWakeupContract> {
        self.option_wakeups
            .iter()
            .find(|option| option.name() == name)
    }
}

/// Built-in source lifecycle metadata needed before a real runtime exists.
///
/// This deliberately stays narrow: the runtime will always need stale-publication suppression when
/// a source instance is replaced or disposed, but only some providers require an extra best-effort
/// in-flight cancellation request on top of that generic generation guard.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SourceLifecycleContract {
    cancellation: SourceCancellationPolicy,
}

impl SourceLifecycleContract {
    pub const fn new(cancellation: SourceCancellationPolicy) -> Self {
        Self { cancellation }
    }

    pub const fn cancellation(self) -> SourceCancellationPolicy {
        self.cancellation
    }
}

/// Whether a built-in source should request explicit cancellation of in-flight work when it is
/// replaced, suspended, or disposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceCancellationPolicy {
    ProviderManaged,
    CancelInFlight,
}

/// Intrinsic recurrent wakeup that the provider guarantees without extra option slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceContractIntrinsicWakeup {
    Timer,
    /// Indicates that the provider fires on its own internal trigger (e.g. an incoming stream
    /// event or OS-level notification) without a compiler-visible option slot.
    ///
    /// # Information loss on conversion
    ///
    /// When a source contract is lowered to this variant, the specific trigger mechanism from the
    /// original contract representation is discarded. For example, `FsWatch`, `SocketConnect`,
    /// `MailboxSubscribe`, `ProcessSpawn`, and `WindowKeyDown` all collapse to this single
    /// variant, making round-tripping from `ProviderDefinedTrigger` back to the original
    /// provider identity impossible without the surrounding `SourceContract` context.
    ProviderDefinedTrigger,
}

/// One provider-defined option slot that proves a recurrent wakeup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceOptionWakeupContract {
    name: &'static str,
    cause: SourceOptionWakeupCause,
}

impl SourceOptionWakeupContract {
    pub const fn new(name: &'static str, cause: SourceOptionWakeupCause) -> Self {
        Self { name, cause }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn cause(self) -> SourceOptionWakeupCause {
        self.cause
    }
}

/// Closed wakeup causes that can be attached to source option slots today.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceOptionWakeupCause {
    RetryPolicy,
    PollingPolicy,
    TriggerSignal,
}

/// Represents the type of a source provider option.
///
/// The recursive structure allows expressing nested container types such as
/// `Signal<List<Int>>` or `Map<Text, List<Float>>`. Construction sites that
/// previously used `Atom` for the inner type should now use
/// `Box::new(SourceContractType::Atom(...))`.
///
/// This is intentionally narrower than user-written HIR type expressions. It records only the
/// closed RFC option shapes the compiler knows today without pretending ordinary expression typing
/// or runtime/provider lowering already exists.
///
/// The `Signal` variant holds a `Box<SourceContractType>` (rather than a flat `SourceTypeAtom`)
/// so that it can represent nested type expressions such as `Signal<List<Int>>`. This makes the
/// type self-referential and therefore non-`Copy`; use `.clone()` where a copy was previously
/// implicit.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum SourceContractType {
    Atom(SourceTypeAtom),
    List(Box<SourceContractType>),
    Map {
        key: SourceTypeAtom, // keys remain atoms (no nested key types)
        value: Box<SourceContractType>,
    },
    /// A reactive signal type whose payload is a full `SourceContractType`, enabling nested
    /// forms like `Signal<List<Int>>` that the former flat `SourceTypeAtom` payload could not
    /// represent.
    Signal(Box<SourceContractType>),
}

impl SourceContractType {
    pub const fn atom(atom: SourceTypeAtom) -> Self {
        Self::Atom(atom)
    }

    pub const fn bool() -> Self {
        Self::Atom(SourceTypeAtom::primitive(PrimitiveType::Bool))
    }

    pub const fn int() -> Self {
        Self::Atom(SourceTypeAtom::primitive(PrimitiveType::Int))
    }

    pub const fn text() -> Self {
        Self::Atom(SourceTypeAtom::primitive(PrimitiveType::Text))
    }

    pub const fn nominal(ty: SourceNominalType) -> Self {
        Self::Atom(SourceTypeAtom::nominal(ty))
    }

    pub const fn parameter(parameter: SourceTypeParameter) -> Self {
        Self::Atom(SourceTypeAtom::parameter(parameter))
    }

    pub fn list(element: SourceTypeAtom) -> Self {
        Self::List(Box::new(SourceContractType::Atom(element)))
    }

    pub fn map(key: SourceTypeAtom, value: SourceTypeAtom) -> Self {
        Self::Map {
            key,
            value: Box::new(SourceContractType::Atom(value)),
        }
    }

    pub fn signal(payload: impl Into<SourceContractType>) -> Self {
        Self::Signal(Box::new(payload.into()))
    }

    pub fn to_kind_expr(self, store: &mut KindStore) -> KindExprId {
        match self {
            Self::Atom(atom) => atom.to_kind_expr(store),
            Self::List(element) => apply_unary_kind_constructor("List", element, store),
            Self::Map { key, value } => apply_binary_kind_constructor("Map", key, value, store),
            Self::Signal(payload) => {
                let constructor = store.add_constructor("Signal".to_owned(), Kind::constructor(1));
                let callee = store.constructor_expr(constructor);
                let argument = payload.to_kind_expr(store);
                store.apply_expr(callee, argument)
            }
        }
    }
}

fn apply_unary_kind_constructor(
    name: &str,
    element: Box<SourceContractType>,
    store: &mut KindStore,
) -> KindExprId {
    let constructor = store.add_constructor(name.to_owned(), Kind::constructor(1));
    let callee = store.constructor_expr(constructor);
    let argument = element.to_kind_expr(store);
    store.apply_expr(callee, argument)
}

fn apply_binary_kind_constructor(
    name: &str,
    key: SourceTypeAtom,
    value: Box<SourceContractType>,
    store: &mut KindStore,
) -> KindExprId {
    let constructor = store.add_constructor(name.to_owned(), Kind::constructor(2));
    let callee = store.constructor_expr(constructor);
    let left = key.to_kind_expr(store);
    let applied_left = store.apply_expr(callee, left);
    let right = value.to_kind_expr(store);
    store.apply_expr(applied_left, right)
}

impl From<SourceTypeAtom> for SourceContractType {
    fn from(atom: SourceTypeAtom) -> Self {
        Self::Atom(atom)
    }
}

impl fmt::Display for SourceContractType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Atom(atom) => write!(f, "{atom}"),
            Self::List(element) => write!(f, "List {element}"),
            Self::Map { key, value } => write!(f, "Map {key} {value}"),
            Self::Signal(payload) => write!(f, "Signal {payload}"),
        }
    }
}

/// Leaf types that can appear inside the current built-in source option contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceTypeAtom {
    Primitive(PrimitiveType),
    Nominal(SourceNominalType),
    Parameter(SourceTypeParameter),
}

impl SourceTypeAtom {
    pub const fn primitive(primitive: PrimitiveType) -> Self {
        Self::Primitive(primitive)
    }

    pub const fn nominal(ty: SourceNominalType) -> Self {
        Self::Nominal(ty)
    }

    pub const fn parameter(parameter: SourceTypeParameter) -> Self {
        Self::Parameter(parameter)
    }

    fn to_kind_expr(self, store: &mut KindStore) -> KindExprId {
        match self {
            Self::Primitive(primitive) => scalar_kind_expr(primitive_name(primitive), store),
            Self::Nominal(ty) => scalar_kind_expr(ty.as_str(), store),
            Self::Parameter(parameter) => {
                let parameter = store.add_parameter(parameter.as_str());
                store.parameter_expr(parameter)
            }
        }
    }
}

impl fmt::Display for SourceTypeAtom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(primitive) => write!(f, "{}", primitive_name(*primitive)),
            Self::Nominal(ty) => write!(f, "{ty}"),
            Self::Parameter(parameter) => write!(f, "{parameter}"),
        }
    }
}

/// RFC-named nominal source contract atoms that are not yet first-class builtins in HIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceNominalType {
    DecodeMode,
    Duration,
    FsWatchEvent,
    Path,
    Retry,
    StreamMode,
}

impl SourceNominalType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DecodeMode => "DecodeMode",
            Self::Duration => "Duration",
            Self::FsWatchEvent => "FsWatchEvent",
            Self::Path => "Path",
            Self::Retry => "Retry",
            Self::StreamMode => "StreamMode",
        }
    }
}

impl fmt::Display for SourceNominalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Contract-local type parameters preserved from the RFC source option surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SourceTypeParameter {
    A,
    B,
}

impl SourceTypeParameter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

impl fmt::Display for SourceTypeParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// HTTP_OPTIONS contains Signal variants and therefore cannot be a `const` array (Box::new is
// not usable in const context). It is initialised exactly once via OnceLock.
static HTTP_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn http_options() -> &'static [SourceOptionContract] {
    HTTP_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new(
                "headers",
                SourceContractType::map(
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                ),
            ),
            SourceOptionContract::new(
                "query",
                SourceContractType::map(
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                ),
            ),
            SourceOptionContract::new(
                "body",
                SourceContractType::parameter(SourceTypeParameter::A),
            ),
            SourceOptionContract::new(
                "decode",
                SourceContractType::nominal(SourceNominalType::DecodeMode),
            ),
            SourceOptionContract::new(
                "timeout",
                SourceContractType::nominal(SourceNominalType::Duration),
            ),
            SourceOptionContract::new(
                "retry",
                SourceContractType::nominal(SourceNominalType::Retry),
            ),
            SourceOptionContract::new(
                "refreshOn",
                SourceContractType::signal(SourceTypeAtom::parameter(SourceTypeParameter::B)),
            ),
            SourceOptionContract::new(
                "refreshEvery",
                SourceContractType::nominal(SourceNominalType::Duration),
            ),
            SourceOptionContract::new(
                "activeWhen",
                SourceContractType::signal(SourceTypeAtom::primitive(PrimitiveType::Bool)),
            ),
        ]
    })
}

const HTTP_WAKEUP_OPTIONS: [SourceOptionWakeupContract; 3] = [
    SourceOptionWakeupContract::new("retry", SourceOptionWakeupCause::RetryPolicy),
    SourceOptionWakeupContract::new("refreshOn", SourceOptionWakeupCause::TriggerSignal),
    SourceOptionWakeupContract::new("refreshEvery", SourceOptionWakeupCause::PollingPolicy),
];

static TIMER_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn timer_options() -> &'static [SourceOptionContract] {
    TIMER_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new("immediate", SourceContractType::bool()),
            SourceOptionContract::new(
                "jitter",
                SourceContractType::nominal(SourceNominalType::Duration),
            ),
            SourceOptionContract::new("coalesce", SourceContractType::bool()),
            SourceOptionContract::new(
                "activeWhen",
                SourceContractType::signal(SourceTypeAtom::primitive(PrimitiveType::Bool)),
            ),
        ]
    })
}

static FS_WATCH_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn fs_watch_options() -> &'static [SourceOptionContract] {
    FS_WATCH_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new(
                "events",
                SourceContractType::list(SourceTypeAtom::nominal(SourceNominalType::FsWatchEvent)),
            ),
            SourceOptionContract::new("recursive", SourceContractType::bool()),
        ]
    })
}

static FS_READ_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn fs_read_options() -> &'static [SourceOptionContract] {
    FS_READ_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new(
                "decode",
                SourceContractType::nominal(SourceNominalType::DecodeMode),
            ),
            SourceOptionContract::new(
                "reloadOn",
                SourceContractType::signal(SourceTypeAtom::parameter(SourceTypeParameter::A)),
            ),
            SourceOptionContract::new(
                "debounce",
                SourceContractType::nominal(SourceNominalType::Duration),
            ),
            SourceOptionContract::new("readOnStart", SourceContractType::bool()),
        ]
    })
}

const FS_READ_WAKEUP_OPTIONS: [SourceOptionWakeupContract; 1] = [SourceOptionWakeupContract::new(
    "reloadOn",
    SourceOptionWakeupCause::TriggerSignal,
)];

static SOCKET_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn socket_options() -> &'static [SourceOptionContract] {
    SOCKET_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new(
                "decode",
                SourceContractType::nominal(SourceNominalType::DecodeMode),
            ),
            SourceOptionContract::new("buffer", SourceContractType::int()),
            SourceOptionContract::new("reconnect", SourceContractType::bool()),
            SourceOptionContract::new(
                "heartbeat",
                SourceContractType::nominal(SourceNominalType::Duration),
            ),
            SourceOptionContract::new(
                "activeWhen",
                SourceContractType::signal(SourceTypeAtom::primitive(PrimitiveType::Bool)),
            ),
        ]
    })
}

static PROCESS_OPTIONS_STORAGE: OnceLock<Vec<SourceOptionContract>> = OnceLock::new();

fn process_options() -> &'static [SourceOptionContract] {
    PROCESS_OPTIONS_STORAGE.get_or_init(|| {
        vec![
            SourceOptionContract::new("cwd", SourceContractType::nominal(SourceNominalType::Path)),
            SourceOptionContract::new(
                "env",
                SourceContractType::map(
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                ),
            ),
            SourceOptionContract::new(
                "stdout",
                SourceContractType::nominal(SourceNominalType::StreamMode),
            ),
            SourceOptionContract::new(
                "stderr",
                SourceContractType::nominal(SourceNominalType::StreamMode),
            ),
            SourceOptionContract::new(
                "restartOn",
                SourceContractType::signal(SourceTypeAtom::parameter(SourceTypeParameter::A)),
            ),
        ]
    })
}

const PROCESS_WAKEUP_OPTIONS: [SourceOptionWakeupContract; 1] = [SourceOptionWakeupContract::new(
    "restartOn",
    SourceOptionWakeupCause::TriggerSignal,
)];

static WINDOW_OPTIONS: [SourceOptionContract; 3] = [
    SourceOptionContract::new("capture", SourceContractType::bool()),
    SourceOptionContract::new("repeat", SourceContractType::bool()),
    SourceOptionContract::new("focusOnly", SourceContractType::bool()),
];
static NO_OPTIONS: [SourceOptionContract; 0] = [];

const HTTP_RECURRENCE: SourceRecurrenceContract =
    SourceRecurrenceContract::new(None, &HTTP_WAKEUP_OPTIONS);
const TIMER_RECURRENCE: SourceRecurrenceContract =
    SourceRecurrenceContract::new(Some(SourceContractIntrinsicWakeup::Timer), &[]);
const FS_WATCH_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(
    Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger),
    &[],
);
const FS_READ_RECURRENCE: SourceRecurrenceContract =
    SourceRecurrenceContract::new(None, &FS_READ_WAKEUP_OPTIONS);
const SOCKET_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(
    Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger),
    &[],
);
const MAILBOX_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(
    Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger),
    &[],
);
const PROCESS_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(
    Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger),
    &PROCESS_WAKEUP_OPTIONS,
);
const WINDOW_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(
    Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger),
    &[],
);
const STATIC_RECURRENCE: SourceRecurrenceContract = SourceRecurrenceContract::new(None, &[]);
const HTTP_LIFECYCLE: SourceLifecycleContract =
    SourceLifecycleContract::new(SourceCancellationPolicy::CancelInFlight);
const TIMER_LIFECYCLE: SourceLifecycleContract =
    SourceLifecycleContract::new(SourceCancellationPolicy::ProviderManaged);
const STATIC_LIFECYCLE: SourceLifecycleContract =
    SourceLifecycleContract::new(SourceCancellationPolicy::ProviderManaged);
const STREAM_LIFECYCLE: SourceLifecycleContract =
    SourceLifecycleContract::new(SourceCancellationPolicy::ProviderManaged);

fn scalar_kind_expr(name: &str, store: &mut KindStore) -> KindExprId {
    let constructor = store.add_constructor(name.to_owned(), Kind::Type);
    store.constructor_expr(constructor)
}

fn primitive_name(primitive: PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Int => "Int",
        PrimitiveType::Float => "Float",
        PrimitiveType::Decimal => "Decimal",
        PrimitiveType::BigInt => "BigInt",
        PrimitiveType::Bool => "Bool",
        PrimitiveType::Text => "Text",
        PrimitiveType::Unit => "Unit",
        PrimitiveType::Bytes => "Bytes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KindChecker;

    #[test]
    fn exposes_http_source_option_contract_types() {
        let contract = BuiltinSourceProvider::HttpGet.contract();

        assert_eq!(contract.provider(), BuiltinSourceProvider::HttpGet);
        assert_eq!(
            contract.option("headers").map(|option| option.ty()),
            Some(SourceContractType::map(
                SourceTypeAtom::primitive(PrimitiveType::Text),
                SourceTypeAtom::primitive(PrimitiveType::Text),
            ))
        );
        assert_eq!(
            contract.option("body").map(|option| option.ty()),
            Some(SourceContractType::parameter(SourceTypeParameter::A))
        );
        assert_eq!(
            contract.option("refreshOn").map(|option| option.ty()),
            Some(SourceContractType::signal(SourceTypeAtom::parameter(
                SourceTypeParameter::B,
            )))
        );
        assert_eq!(
            contract.option("timeout").map(|option| option.ty()),
            Some(SourceContractType::nominal(SourceNominalType::Duration))
        );
        assert_eq!(
            contract.option("retry").map(|option| option.ty()),
            Some(SourceContractType::nominal(SourceNominalType::Retry))
        );
    }

    #[test]
    fn uses_domain_quantity_vocabulary_for_built_in_contracts() {
        let timer = BuiltinSourceProvider::TimerEvery.contract();
        let http = BuiltinSourceProvider::HttpGet.contract();

        assert!(timer.option("jitter").is_some());
        assert!(timer.option("jitterMs").is_none());
        assert!(http.option("refreshEvery").is_some());
        assert!(http.option("refreshEveryMs").is_none());
    }

    #[test]
    fn exposes_builtin_recurrence_contract_metadata() {
        let http = BuiltinSourceProvider::HttpGet.contract();
        let timer = BuiltinSourceProvider::TimerEvery.contract();
        let fs_read = BuiltinSourceProvider::FsRead.contract();
        let process = BuiltinSourceProvider::ProcessSpawn.contract();

        assert_eq!(http.intrinsic_wakeup(), None);
        assert_eq!(
            http.wakeup_option("retry").map(|option| option.cause()),
            Some(SourceOptionWakeupCause::RetryPolicy)
        );
        assert_eq!(
            http.wakeup_option("refreshEvery")
                .map(|option| option.cause()),
            Some(SourceOptionWakeupCause::PollingPolicy)
        );
        assert_eq!(
            http.wakeup_option("refreshOn").map(|option| option.cause()),
            Some(SourceOptionWakeupCause::TriggerSignal)
        );

        assert_eq!(
            timer.intrinsic_wakeup(),
            Some(SourceContractIntrinsicWakeup::Timer)
        );
        assert_eq!(timer.wakeup_option("immediate"), None);

        assert_eq!(fs_read.intrinsic_wakeup(), None);
        assert_eq!(
            fs_read
                .wakeup_option("reloadOn")
                .map(|option| option.cause()),
            Some(SourceOptionWakeupCause::TriggerSignal)
        );

        assert_eq!(
            process.intrinsic_wakeup(),
            Some(SourceContractIntrinsicWakeup::ProviderDefinedTrigger)
        );
        assert_eq!(
            process
                .wakeup_option("restartOn")
                .map(|option| option.cause()),
            Some(SourceOptionWakeupCause::TriggerSignal)
        );
    }

    #[test]
    fn exposes_builtin_source_lifecycle_contract_metadata() {
        let http = BuiltinSourceProvider::HttpGet.contract();
        let fs_read = BuiltinSourceProvider::FsRead.contract();
        let timer = BuiltinSourceProvider::TimerEvery.contract();
        let socket = BuiltinSourceProvider::SocketConnect.contract();

        assert_eq!(
            http.lifecycle().cancellation(),
            SourceCancellationPolicy::CancelInFlight
        );
        assert_eq!(
            fs_read.lifecycle().cancellation(),
            SourceCancellationPolicy::CancelInFlight
        );
        assert_eq!(
            timer.lifecycle().cancellation(),
            SourceCancellationPolicy::ProviderManaged
        );
        assert_eq!(
            socket.lifecycle().cancellation(),
            SourceCancellationPolicy::ProviderManaged
        );
    }

    #[test]
    fn all_source_option_contract_types_are_type_kinded() {
        for provider in BuiltinSourceProvider::ALL {
            for option in provider.contract().options() {
                let mut store = KindStore::default();
                let expr = option.ty().to_kind_expr(&mut store);
                KindChecker
                    .expect_kind(&store, expr, &Kind::Type)
                    .unwrap_or_else(|error| {
                        panic!(
                            "{}::{} should stay Type-kinded, got {:?}",
                            provider.key(),
                            option.name(),
                            error.kind()
                        )
                    });
            }
        }
    }
}
