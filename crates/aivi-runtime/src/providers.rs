use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{BufRead, BufReader},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use aivi_backend::{DetachedRuntimeValue, RuntimeCallable, RuntimeValue};
use aivi_hir as hir;
use aivi_typing::BuiltinSourceProvider;
use url::Url;

use crate::startup::DetachedRuntimePublicationPort;
use crate::{
    CancellationObserver, EvaluatedSourceConfig, LinkedSourceLifecycleAction,
    RuntimeSourceProvider, SourceInstanceId, SourceLifecycleActionKind,
    source_decode::{
        ExternalSourceValue, SourceDecodeError, decode_external, encode_runtime_json,
        parse_json_text, validate_supported_program,
    },
};

// TODO: `MailboxHub` currently never shrinks `subscribers` unless a subscriber disconnects or is
// explicitly unsubscribed. If sources are activated and suspended repeatedly, the inner `Vec` for
// each mailbox key can grow unboundedly. Future work should either use weak references so that
// dead subscribers are collected automatically on the next `publish` call, or expose an explicit
// `remove` / `unsubscribe` operation that callers invoke when a source is deactivated.
#[derive(Default)]
struct MailboxHub {
    next_id: u64,
    subscribers: BTreeMap<Box<str>, Vec<MailboxSubscriber>>,
}

struct MailboxSubscriber {
    id: u64,
    sender: SyncSender<Box<str>>,
}

impl MailboxHub {
    fn subscribe(&mut self, mailbox: &str, buffer: usize) -> (u64, mpsc::Receiver<Box<str>>) {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("mailbox subscriber ids should not overflow");
        let (sender, receiver) = mpsc::sync_channel(buffer.max(1));
        self.subscribers
            .entry(mailbox.into())
            .or_default()
            .push(MailboxSubscriber { id, sender });
        (id, receiver)
    }

    fn unsubscribe(&mut self, mailbox: &str, id: u64) {
        let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
            return;
        };
        subscribers.retain(|subscriber| subscriber.id != id);
        if subscribers.is_empty() {
            self.subscribers.remove(mailbox);
        }
    }

    fn publish(&mut self, mailbox: &str, message: &str) -> Result<(), MailboxPublishError> {
        let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
            return Ok(());
        };
        let mut full = false;
        subscribers.retain(
            |subscriber| match subscriber.sender.try_send(message.into()) {
                Ok(()) => true,
                Err(TrySendError::Disconnected(_)) => false,
                Err(TrySendError::Full(_)) => {
                    full = true;
                    true
                }
            },
        );
        if subscribers.is_empty() {
            self.subscribers.remove(mailbox);
        }
        if full {
            Err(MailboxPublishError::BufferFull {
                mailbox: mailbox.into(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailboxPublishError {
    BufferFull { mailbox: Box<str> },
}

impl fmt::Display for MailboxPublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferFull { mailbox } => write!(
                f,
                "mailbox `{mailbox}` cannot accept a new message because at least one subscriber buffer is full"
            ),
        }
    }
}

impl std::error::Error for MailboxPublishError {}

#[derive(Clone)]
enum ActiveProviderState {
    Passive {
        provider: RuntimeSourceProvider,
        /// Stop flag set to `true` when this source instance is disposed.
        /// Worker threads check this flag before each iteration so they exit promptly
        /// when the source is removed, even if the port cancellation signal has not yet
        /// propagated through the scheduler.
        stop: Arc<AtomicBool>,
    },
    Mailbox {
        provider: RuntimeSourceProvider,
        mailbox: Box<str>,
        subscriber_id: u64,
        /// Stop flag for the mailbox worker thread; set to `true` on disposal.
        stop: Arc<AtomicBool>,
    },
    Window {
        provider: RuntimeSourceProvider,
        output: WindowKeyOutputPlan,
        allow_repeat: bool,
        port: DetachedRuntimePublicationPort,
    },
}

impl ActiveProviderState {
    fn provider(&self) -> &RuntimeSourceProvider {
        match self {
            Self::Passive { provider, .. }
            | Self::Mailbox { provider, .. }
            | Self::Window { provider, .. } => provider,
        }
    }
}

#[derive(Clone, Default)]
pub struct SourceProviderManager {
    active: BTreeMap<SourceInstanceId, ActiveProviderState>,
    mailboxes: Arc<Mutex<MailboxHub>>,
}

impl SourceProviderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_provider(&self, instance: SourceInstanceId) -> Option<&RuntimeSourceProvider> {
        self.active
            .get(&instance)
            .map(ActiveProviderState::provider)
    }

    pub fn publish_mailbox_message(
        &self,
        mailbox: &str,
        message: &str,
    ) -> Result<(), MailboxPublishError> {
        self.mailboxes
            .lock()
            .expect("mailbox hub mutex should not be poisoned")
            .publish(mailbox, message)
    }

    pub fn dispatch_window_key_event(&mut self, event: WindowKeyEvent) {
        for state in self.active.values() {
            let ActiveProviderState::Window {
                output,
                allow_repeat,
                port,
                ..
            } = state
            else {
                continue;
            };
            if event.repeated && !allow_repeat {
                continue;
            }
            let Ok(Some(value)) = output.value_for_key(&event.name) else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
        }
    }

    pub fn apply_actions(
        &mut self,
        actions: &[LinkedSourceLifecycleAction],
    ) -> Result<(), SourceProviderExecutionError> {
        for action in actions {
            match action {
                LinkedSourceLifecycleAction::Activate {
                    instance,
                    port,
                    config,
                }
                | LinkedSourceLifecycleAction::Reconfigure {
                    instance,
                    port,
                    config,
                } => self.start_provider(action.kind(), *instance, config, port.clone())?,
                LinkedSourceLifecycleAction::Suspend { instance } => {
                    self.remove_active(*instance);
                }
            }
        }
        Ok(())
    }

    fn start_provider(
        &mut self,
        action_kind: SourceLifecycleActionKind,
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
        port: DetachedRuntimePublicationPort,
    ) -> Result<(), SourceProviderExecutionError> {
        self.remove_active(instance);
        let state = match &config.provider {
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerEvery) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerEvery, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_timer_every(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerAfter) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerAfter, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_timer_after(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::HttpGet) => {
                let plan = HttpPlan::parse(instance, BuiltinSourceProvider::HttpGet, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_http_worker(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::HttpPost) => {
                let plan = HttpPlan::parse(instance, BuiltinSourceProvider::HttpPost, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_http_worker(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::FsRead) => {
                let plan = FsReadPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                if action_kind == SourceLifecycleActionKind::Reconfigure || plan.read_on_start {
                    spawn_fs_read_worker(port, plan, stop.clone());
                }
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::FsWatch) => {
                let plan = FsWatchPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_fs_watch_worker(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::SocketConnect) => {
                let plan = SocketPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_socket_worker(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::MailboxSubscribe) => {
                let plan = MailboxPlan::parse(instance, config)?;
                let (subscriber_id, receiver) = self
                    .mailboxes
                    .lock()
                    .expect("mailbox hub mutex should not be poisoned")
                    .subscribe(&plan.mailbox, plan.buffer);
                let stop = Arc::new(AtomicBool::new(false));
                spawn_mailbox_worker(port, plan.clone(), receiver, stop.clone());
                ActiveProviderState::Mailbox {
                    provider: config.provider.clone(),
                    mailbox: plan.mailbox,
                    subscriber_id,
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ProcessSpawn) => {
                let plan = ProcessPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                spawn_process_worker(port, plan, stop.clone());
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::WindowKeyDown) => {
                let plan = WindowKeyDownPlan::parse(instance, config)?;
                ActiveProviderState::Window {
                    provider: config.provider.clone(),
                    output: plan.output,
                    allow_repeat: plan.allow_repeat,
                    port,
                }
            }
            provider => {
                return Err(SourceProviderExecutionError::UnsupportedProvider {
                    instance,
                    provider: provider.clone(),
                });
            }
        };
        self.active.insert(instance, state);
        Ok(())
    }

    fn remove_active(&mut self, instance: SourceInstanceId) {
        let Some(state) = self.active.remove(&instance) else {
            return;
        };
        // Signal the worker thread to stop. The worker will observe this flag
        // on its next iteration and exit cleanly without waiting for the port
        // cancellation to propagate through the scheduler.
        match &state {
            ActiveProviderState::Passive { stop, .. } => {
                stop.store(true, Ordering::Release);
            }
            ActiveProviderState::Mailbox {
                mailbox,
                subscriber_id,
                stop,
                ..
            } => {
                stop.store(true, Ordering::Release);
                self.mailboxes
                    .lock()
                    .expect("mailbox hub mutex should not be poisoned")
                    .unsubscribe(mailbox, *subscriber_id);
            }
            ActiveProviderState::Window { .. } => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowKeyEvent {
    pub name: Box<str>,
    pub repeated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderExecutionError {
    UnsupportedProvider {
        instance: SourceInstanceId,
        provider: RuntimeSourceProvider,
    },
    MissingDecodeProgram {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
    },
    UnsupportedDecodeProgram {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
    UnsupportedProviderShape {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
    InvalidArgumentCount {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        expected: usize,
        found: usize,
    },
    InvalidArgument {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        index: usize,
        expected: Box<str>,
        value: RuntimeValue,
    },
    InvalidOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
        expected: Box<str>,
        value: RuntimeValue,
    },
    UnsupportedOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
    },
    ZeroTimerInterval {
        instance: SourceInstanceId,
    },
    StartFailed {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
}

impl fmt::Display for SourceProviderExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProvider { instance, provider } => write!(
                f,
                "source instance {} uses unsupported runtime provider {:?}",
                instance.as_raw(),
                provider
            ),
            Self::MissingDecodeProgram { instance, provider } => write!(
                f,
                "source instance {} provider {} is missing its decode program",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedDecodeProgram {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} cannot execute its decode program: {detail}",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedProviderShape {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} cannot execute this source shape: {detail}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidArgumentCount {
                instance,
                provider,
                expected,
                found,
            } => write!(
                f,
                "source instance {} provider {} expects {expected} argument(s), found {found}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidArgument {
                instance,
                provider,
                index,
                expected,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid argument {index}; expected {expected}, found {value}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidOption {
                instance,
                provider,
                option_name,
                expected,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid `{option_name}` option; expected {expected}, found {value}",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedOption {
                instance,
                provider,
                option_name,
            } => write!(
                f,
                "source instance {} provider {} does not execute `{option_name}` yet",
                instance.as_raw(),
                provider.key()
            ),
            Self::ZeroTimerInterval { instance } => write!(
                f,
                "source instance {} cannot execute a timer with a zero or negative interval; durations must be positive",
                instance.as_raw()
            ),
            Self::StartFailed {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} failed to start: {detail}",
                instance.as_raw(),
                provider.key()
            ),
        }
    }
}

impl std::error::Error for SourceProviderExecutionError {}

#[derive(Clone, Copy)]
struct TimerPlan {
    delay: Duration,
    immediate: bool,
}

impl TimerPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let delay = parse_duration(instance, provider, 0, &config.arguments[0])?;
        // Reject zero and negative durations: a zero interval would spin the worker thread at 100%
        // CPU, and negative durations are not representable as `std::time::Duration` (they would
        // be silently clamped to zero by the `i64 as u64` cast in `parse_duration`).
        if delay.is_zero() {
            return Err(SourceProviderExecutionError::ZeroTimerInterval { instance });
        }
        let mut immediate = false;
        for option in &config.options {
            match option.option_name.as_ref() {
                "immediate" => {
                    immediate = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "coalesce" => {
                    if !parse_bool(instance, provider, &option.option_name, &option.value)? {
                        return Err(SourceProviderExecutionError::UnsupportedOption {
                            instance,
                            provider,
                            option_name: option.option_name.clone(),
                        });
                    }
                }
                "activeWhen" => {}
                "jitter" => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self { delay, immediate })
    }
}

#[derive(Clone, Copy)]
enum PayloadDecodeMode {
    Text,
    Json,
}

#[derive(Clone)]
struct RequestResultPlan {
    decode: hir::SourceDecodeProgram,
    success_mode: PayloadDecodeMode,
    error: ErrorPlan,
}

#[derive(Clone)]
enum ErrorPlan {
    Text,
    Sum { variants: Box<[SumErrorVariant]> },
}

#[derive(Clone)]
struct SumErrorVariant {
    name: Box<str>,
    payload: ErrorPayloadKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorPayloadKind {
    None,
    Text,
    Int,
}

impl RequestResultPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        let hir::DecodeProgramStep::Result { error, value } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "request and stream providers currently require `Signal (Result E A)` outputs"
                        .into(),
            });
        };
        let success_mode = if matches!(
            decode.step(*value),
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            }
        ) {
            PayloadDecodeMode::Text
        } else {
            PayloadDecodeMode::Json
        };
        let error = ErrorPlan::from_step(instance, provider, &decode, *error)?;
        Ok(Self {
            decode,
            success_mode,
            error,
        })
    }

    fn success_from_text(&self, text: &str) -> Result<RuntimeValue, SourceDecodeError> {
        let payload = match self.success_mode {
            PayloadDecodeMode::Text => ExternalSourceValue::Text(text.into()),
            PayloadDecodeMode::Json => parse_json_text(text)?,
        };
        decode_external(
            &self.decode,
            &ExternalSourceValue::variant_with_payload("Ok", payload),
        )
    }

    fn error_value(
        &self,
        kind: TextSourceErrorKind,
        message: &str,
    ) -> Result<RuntimeValue, Box<str>> {
        let payload = self.error.payload_for(kind, message)?;
        decode_external(
            &self.decode,
            &ExternalSourceValue::variant_with_payload("Err", payload),
        )
        .map_err(|error| error.to_string().into_boxed_str())
    }
}

impl ErrorPlan {
    fn from_step(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        decode: &hir::SourceDecodeProgram,
        step_id: hir::DecodeProgramStepId,
    ) -> Result<Self, SourceProviderExecutionError> {
        match decode.step(step_id) {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                let mut supported = Vec::with_capacity(variants.len());
                for variant in variants {
                    let payload = match variant.payload {
                        None => ErrorPayloadKind::None,
                        Some(payload) => match decode.step(payload) {
                            hir::DecodeProgramStep::Scalar {
                                scalar: aivi_typing::PrimitiveType::Text,
                            } => ErrorPayloadKind::Text,
                            hir::DecodeProgramStep::Scalar {
                                scalar: aivi_typing::PrimitiveType::Int,
                            } => ErrorPayloadKind::Int,
                            _ => {
                                return Err(
                                    SourceProviderExecutionError::UnsupportedProviderShape {
                                        instance,
                                        provider,
                                        detail: format!(
                                            "result error variant `{}` must be nullary, Text, or Int in the current runtime slice",
                                            variant.name.as_str()
                                        )
                                        .into_boxed_str(),
                                    },
                                );
                            }
                        },
                    };
                    supported.push(SumErrorVariant {
                        name: variant.name.as_str().into(),
                        payload,
                    });
                }
                Ok(Self::Sum {
                    variants: supported.into_boxed_slice(),
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "request and stream provider errors must currently decode as `Text` or an explicit sum"
                        .into(),
            }),
        }
    }

    fn payload_for(
        &self,
        kind: TextSourceErrorKind,
        message: &str,
    ) -> Result<ExternalSourceValue, Box<str>> {
        match self {
            Self::Text => Ok(ExternalSourceValue::Text(message.into())),
            Self::Sum { variants } => {
                for spec in kind.candidates() {
                    let Some(variant) = variants
                        .iter()
                        .find(|variant| variant.name.as_ref() == spec.name)
                    else {
                        continue;
                    };
                    if variant.payload != spec.payload {
                        continue;
                    }
                    return Ok(match spec.payload {
                        ErrorPayloadKind::None => ExternalSourceValue::variant(spec.name),
                        ErrorPayloadKind::Text => ExternalSourceValue::variant_with_payload(
                            spec.name,
                            ExternalSourceValue::Text(message.into()),
                        ),
                        ErrorPayloadKind::Int => ExternalSourceValue::variant_with_payload(
                            spec.name,
                            ExternalSourceValue::Int(spec.int_payload.unwrap_or_default()),
                        ),
                    });
                }
                Err(
                    format!("the current result error type cannot represent a {kind} failure")
                        .into(),
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextSourceErrorKind {
    Timeout,
    Decode,
    Missing,
    Request,
    Connect,
    Mailbox,
}

impl TextSourceErrorKind {
    fn candidates(self) -> &'static [ErrorCandidate] {
        match self {
            Self::Timeout => &TIMEOUT_ERROR_CANDIDATES,
            Self::Decode => &DECODE_ERROR_CANDIDATES,
            Self::Missing => &MISSING_ERROR_CANDIDATES,
            Self::Request => &REQUEST_ERROR_CANDIDATES,
            Self::Connect => &CONNECT_ERROR_CANDIDATES,
            Self::Mailbox => &MAILBOX_ERROR_CANDIDATES,
        }
    }
}

impl fmt::Display for TextSourceErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("timeout"),
            Self::Decode => f.write_str("decode"),
            Self::Missing => f.write_str("missing-file"),
            Self::Request => f.write_str("request"),
            Self::Connect => f.write_str("connect"),
            Self::Mailbox => f.write_str("mailbox"),
        }
    }
}

#[derive(Clone, Copy)]
struct ErrorCandidate {
    name: &'static str,
    payload: ErrorPayloadKind,
    int_payload: Option<i64>,
}

impl ErrorCandidate {
    const fn none(name: &'static str) -> Self {
        Self {
            name,
            payload: ErrorPayloadKind::None,
            int_payload: None,
        }
    }

    const fn text(name: &'static str) -> Self {
        Self {
            name,
            payload: ErrorPayloadKind::Text,
            int_payload: None,
        }
    }
}

const TIMEOUT_ERROR_CANDIDATES: [ErrorCandidate; 5] = [
    ErrorCandidate::none("Timeout"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("TransportFailure"),
    ErrorCandidate::text("Error"),
];

const DECODE_ERROR_CANDIDATES: [ErrorCandidate; 3] = [
    ErrorCandidate::text("DecodeFailure"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("Error"),
];

const MISSING_ERROR_CANDIDATES: [ErrorCandidate; 3] = [
    ErrorCandidate::none("Missing"),
    ErrorCandidate::none("NotFound"),
    ErrorCandidate::text("Error"),
];

const REQUEST_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("TransportFailure"),
    ErrorCandidate::text("Error"),
];

const CONNECT_ERROR_CANDIDATES: [ErrorCandidate; 3] = [
    ErrorCandidate::text("ConnectFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("Error"),
];

const MAILBOX_ERROR_CANDIDATES: [ErrorCandidate; 2] = [
    ErrorCandidate::text("MailboxFailure"),
    ErrorCandidate::text("Error"),
];

#[derive(Clone)]
struct HttpPlan {
    provider: BuiltinSourceProvider,
    url: Box<str>,
    headers: Box<[(Box<str>, Box<str>)]>,
    body: Option<Box<str>>,
    timeout: Option<Duration>,
    refresh_every: Option<Duration>,
    retry_attempts: u32,
    result: RequestResultPlan,
}

impl HttpPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let base_url = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut url = Url::parse(base_url.as_ref()).map_err(|error| {
            SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("invalid HTTP URL `{base_url}`: {error}").into_boxed_str(),
            }
        })?;
        let mut headers = Vec::new();
        let mut body = None;
        let mut timeout = None;
        let mut refresh_every = None;
        let mut retry_attempts = 0;
        for option in &config.options {
            match option.option_name.as_ref() {
                "headers" => {
                    headers =
                        parse_text_map(instance, provider, &option.option_name, &option.value)?;
                }
                "query" => {
                    for (key, value) in
                        parse_text_map(instance, provider, &option.option_name, &option.value)?
                    {
                        url.query_pairs_mut().append_pair(&key, &value);
                    }
                }
                "body" => {
                    if provider == BuiltinSourceProvider::HttpGet {
                        return Err(SourceProviderExecutionError::UnsupportedOption {
                            instance,
                            provider,
                            option_name: option.option_name.clone(),
                        });
                    }
                    body = Some(encode_runtime_body(instance, provider, &option.value)?);
                }
                "timeout" => {
                    timeout = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "retry" => {
                    retry_attempts =
                        parse_retry(instance, provider, &option.option_name, &option.value)?;
                }
                "refreshEvery" => {
                    refresh_every = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "decode" | "refreshOn" | "activeWhen" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            provider,
            url: url.to_string().into_boxed_str(),
            headers: headers.into_boxed_slice(),
            body,
            timeout,
            refresh_every,
            retry_attempts,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct FsReadPlan {
    path: PathBuf,
    debounce: Duration,
    read_on_start: bool,
    result: RequestResultPlan,
}

impl FsReadPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::FsRead;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut debounce = Duration::ZERO;
        let mut read_on_start = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "debounce" => {
                    debounce = parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "readOnStart" => {
                    read_on_start =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "decode" | "reloadOn" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            path: PathBuf::from(path.as_ref()),
            debounce,
            read_on_start,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
enum NamedEventOutputPlan {
    Text,
    Variants {
        decode: hir::SourceDecodeProgram,
        variants: BTreeSet<Box<str>>,
    },
}

impl NamedEventOutputPlan {
    fn parse_payloadless_variants(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        match decode.root_step() {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                let mut names = BTreeSet::new();
                for variant in variants {
                    if variant.payload.is_some() {
                        return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                            instance,
                            provider,
                            detail: format!(
                                "event variant `{}` must be payloadless or the target must be `Text`",
                                variant.name.as_str()
                            )
                            .into_boxed_str(),
                        });
                    }
                    names.insert(variant.name.as_str().into());
                }
                Ok(Self::Variants {
                    decode,
                    variants: names,
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "event providers currently decode to `Text` or a payloadless sum".into(),
            }),
        }
    }

    fn value_for_name(&self, name: &str) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        match self {
            Self::Text => Ok(Some(RuntimeValue::Text(name.into()))),
            Self::Variants { decode, variants } => {
                if !variants.contains(name) {
                    return Ok(None);
                }
                decode_external(decode, &ExternalSourceValue::variant(name)).map(Some)
            }
        }
    }
}

#[derive(Clone)]
struct FsWatchPlan {
    path: PathBuf,
    events: BTreeSet<Box<str>>,
    output: NamedEventOutputPlan,
}

impl FsWatchPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::FsWatch;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut events = ["Created", "Changed", "Deleted"]
            .into_iter()
            .map(Into::into)
            .collect::<BTreeSet<Box<str>>>();
        for option in &config.options {
            match option.option_name.as_ref() {
                "events" => {
                    events = parse_named_variants(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "recursive" => {
                    if parse_bool(instance, provider, &option.option_name, &option.value)? {
                        return Err(SourceProviderExecutionError::UnsupportedOption {
                            instance,
                            provider,
                            option_name: option.option_name.clone(),
                        });
                    }
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            path: PathBuf::from(path.as_ref()),
            events,
            output: NamedEventOutputPlan::parse_payloadless_variants(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct SocketPlan {
    host: Box<str>,
    port: u16,
    buffer: usize,
    reconnect: bool,
    result: RequestResultPlan,
}

impl SocketPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::SocketConnect;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let url = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let url = Url::parse(url.as_ref()).map_err(|error| {
            SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("invalid socket URL `{url}`: {error}").into_boxed_str(),
            }
        })?;
        if url.scheme() != "tcp" {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "socket.connect currently supports only `tcp://host:port` URLs, found `{}`",
                    url.scheme()
                )
                .into_boxed_str(),
            });
        }
        let host = url
            .host_str()
            .ok_or(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: "socket.connect requires a host".into(),
            })?;
        let port = url
            .port()
            .ok_or(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: "socket.connect requires an explicit port".into(),
            })?;
        let mut buffer = 4096usize;
        let mut reconnect = false;
        for option in &config.options {
            match option.option_name.as_ref() {
                "buffer" => {
                    buffer = parse_nonnegative_int(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? as usize;
                }
                "reconnect" => {
                    reconnect = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "decode" | "activeWhen" => {}
                "heartbeat" => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            host: host.into(),
            port,
            buffer,
            reconnect,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct MailboxPlan {
    mailbox: Box<str>,
    buffer: usize,
    result: RequestResultPlan,
}

impl MailboxPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::MailboxSubscribe;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let mailbox = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut buffer = 64usize;
        for option in &config.options {
            match option.option_name.as_ref() {
                "buffer" => {
                    buffer = parse_nonnegative_int(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? as usize;
                }
                "decode" | "activeWhen" => {}
                "reconnect" | "heartbeat" => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            mailbox,
            buffer,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessStreamMode {
    Ignore,
    Lines,
}

#[derive(Clone)]
struct ProcessPlan {
    command: Box<str>,
    args: Box<[Box<str>]>,
    cwd: Option<PathBuf>,
    env: Box<[(Box<str>, Box<str>)]>,
    events: ProcessEventPlan,
}

impl ProcessPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ProcessSpawn;
        if !(1..=2).contains(&config.arguments.len()) {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 2,
                found: config.arguments.len(),
            });
        }
        let command = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let args = if config.arguments.len() == 2 {
            parse_text_list(instance, provider, 1, &config.arguments[1])?
        } else {
            Vec::new()
        };
        let mut cwd = None;
        let mut env = Vec::new();
        let mut stdout = ProcessStreamMode::Ignore;
        let mut stderr = ProcessStreamMode::Ignore;
        for option in &config.options {
            match option.option_name.as_ref() {
                "cwd" => {
                    let cwd_text =
                        parse_text_option(instance, provider, &option.option_name, &option.value)?;
                    cwd = Some(PathBuf::from(cwd_text.as_ref()));
                }
                "env" => {
                    env = parse_text_map(instance, provider, &option.option_name, &option.value)?;
                }
                "stdout" => {
                    stdout =
                        parse_stream_mode(instance, provider, &option.option_name, &option.value)?;
                }
                "stderr" => {
                    stderr =
                        parse_stream_mode(instance, provider, &option.option_name, &option.value)?;
                }
                "restartOn" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        let events = ProcessEventPlan::parse(instance, config)?;
        if stdout == ProcessStreamMode::Lines && events.stdout.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stdout: Lines requires a `Stdout` event variant in the source output type"
                    .into(),
            });
        }
        if stderr == ProcessStreamMode::Lines && events.stderr.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stderr: Lines requires a `Stderr` event variant in the source output type"
                    .into(),
            });
        }
        Ok(Self {
            command,
            args: args.into_boxed_slice(),
            cwd,
            env: env.into_boxed_slice(),
            events,
        })
    }
}

#[derive(Clone)]
struct ProcessEventPlan {
    decode: hir::SourceDecodeProgram,
    spawned: Option<ProcessVariantPlan>,
    stdout: Option<ProcessVariantPlan>,
    stderr: Option<ProcessVariantPlan>,
    exited: Option<ProcessVariantPlan>,
    failed: Option<ProcessVariantPlan>,
}

#[derive(Clone)]
struct ProcessVariantPlan {
    variant: Box<str>,
    payload: ProcessPayloadKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessPayloadKind {
    None,
    Text,
    Int,
}

impl ProcessEventPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ProcessSpawn;
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        let hir::DecodeProgramStep::Sum { variants, .. } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "process.spawn currently requires a sum-shaped `ProcessEvent` output"
                    .into(),
            });
        };
        let mut plan = Self {
            decode: decode.clone(),
            spawned: None,
            stdout: None,
            stderr: None,
            exited: None,
            failed: None,
        };
        for variant in variants {
            let payload = match variant.payload {
                None => ProcessPayloadKind::None,
                Some(step) => match decode.step(step) {
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    } => ProcessPayloadKind::Text,
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Int,
                    } => ProcessPayloadKind::Int,
                    _ => continue,
                },
            };
            let entry = Some(ProcessVariantPlan {
                variant: variant.name.as_str().into(),
                payload,
            });
            match variant.name.as_str() {
                "Spawned" => plan.spawned = entry,
                "Stdout" => plan.stdout = entry,
                "Stderr" => plan.stderr = entry,
                "Exited" => plan.exited = entry,
                "Failed" => plan.failed = entry,
                _ => {}
            }
        }
        Ok(plan)
    }

    fn spawned_value(&self) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        self.variant_value(self.spawned.as_ref(), None)
    }

    fn stdout_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        self.variant_value(
            self.stdout.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn stderr_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        self.variant_value(
            self.stderr.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn exited_value(&self, code: i64) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        self.variant_value(self.exited.as_ref(), Some(ExternalSourceValue::Int(code)))
    }

    fn failed_value(&self, message: &str) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        self.variant_value(
            self.failed.as_ref(),
            Some(ExternalSourceValue::Text(message.into())),
        )
    }

    fn variant_value(
        &self,
        plan: Option<&ProcessVariantPlan>,
        payload: Option<ExternalSourceValue>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        let raw = match (plan.payload, payload) {
            (ProcessPayloadKind::None, _) => ExternalSourceValue::variant(plan.variant.as_ref()),
            (ProcessPayloadKind::Text, Some(payload @ ExternalSourceValue::Text(_)))
            | (ProcessPayloadKind::Int, Some(payload @ ExternalSourceValue::Int(_))) => {
                ExternalSourceValue::variant_with_payload(plan.variant.as_ref(), payload)
            }
            _ => return Ok(None),
        };
        decode_external(&self.decode, &raw).map(Some)
    }
}

#[derive(Clone)]
struct WindowKeyDownPlan {
    allow_repeat: bool,
    output: WindowKeyOutputPlan,
}

impl WindowKeyDownPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::WindowKeyDown;
        if !config.arguments.is_empty() {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 0,
                found: config.arguments.len(),
            });
        }
        let mut capture = false;
        let mut focus_only = true;
        let mut allow_repeat = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "capture" => {
                    capture = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "focusOnly" => {
                    focus_only =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "repeat" => {
                    allow_repeat =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        if capture {
            return Err(SourceProviderExecutionError::UnsupportedOption {
                instance,
                provider,
                option_name: "capture".into(),
            });
        }
        if !focus_only {
            return Err(SourceProviderExecutionError::UnsupportedOption {
                instance,
                provider,
                option_name: "focusOnly".into(),
            });
        }
        Ok(Self {
            allow_repeat,
            output: WindowKeyOutputPlan::parse(instance, config)?,
        })
    }
}

#[derive(Clone)]
enum WindowKeyOutputPlan {
    Text,
    NamedVariants {
        decode: hir::SourceDecodeProgram,
        variants: BTreeSet<Box<str>>,
    },
    WrappedTextVariant {
        decode: hir::SourceDecodeProgram,
        variant: Box<str>,
    },
}

impl WindowKeyOutputPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::WindowKeyDown;
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        match decode.root_step() {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                if let Some(variant) = variants.iter().find_map(|variant| {
                    let Some(payload) = variant.payload else {
                        return None;
                    };
                    if matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Text,
                        }
                    ) {
                        Some(variant.name.as_str().into())
                    } else {
                        None
                    }
                }) {
                    return Ok(Self::WrappedTextVariant { decode, variant });
                }
                let mut names = BTreeSet::new();
                for variant in variants {
                    if variant.payload.is_some() {
                        return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                            instance,
                            provider,
                            detail: "window.keyDown sum outputs must be payloadless key variants or one text payload wrapper".into(),
                        });
                    }
                    names.insert(variant.name.as_str().into());
                }
                Ok(Self::NamedVariants {
                    decode,
                    variants: names,
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "window.keyDown currently decodes to `Text`, a payloadless key sum, or one text-wrapping key constructor".into(),
            }),
        }
    }

    fn value_for_key(&self, key: &str) -> Result<Option<RuntimeValue>, SourceDecodeError> {
        match self {
            Self::Text => Ok(Some(RuntimeValue::Text(key.into()))),
            Self::NamedVariants { decode, variants } => {
                if !variants.contains(key) {
                    return Ok(None);
                }
                decode_external(decode, &ExternalSourceValue::variant(key)).map(Some)
            }
            Self::WrappedTextVariant { decode, variant } => decode_external(
                decode,
                &ExternalSourceValue::variant_with_payload(
                    variant.as_ref(),
                    ExternalSourceValue::Text(key.into()),
                ),
            )
            .map(Some),
        }
    }
}

fn spawn_timer_every(port: DetachedRuntimePublicationPort, plan: TimerPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        if plan.immediate && port.publish(DetachedRuntimeValue::unit()).is_err() {
            return;
        }
        while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
            thread::sleep(plan.delay);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                break;
            }
            if port.publish(DetachedRuntimeValue::unit()).is_err() {
                break;
            }
        }
    });
}

fn spawn_timer_after(port: DetachedRuntimePublicationPort, plan: TimerPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        if !plan.immediate {
            thread::sleep(plan.delay);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
        }
        let _ = port.publish(DetachedRuntimeValue::unit());
    });
}

fn spawn_http_worker(port: DetachedRuntimePublicationPort, plan: HttpPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let Some(value) = execute_http_cycle(&plan, &port) else {
                return;
            };
            if stop.load(Ordering::Acquire) {
                return;
            }
            if port
                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                .is_err()
            {
                return;
            }
            let Some(refresh_every) = plan.refresh_every else {
                return;
            };
            if stop.load(Ordering::Acquire) || sleep_with_cancellation(refresh_every, &port) {
                return;
            }
        }
    });
}

fn execute_http_cycle(
    plan: &HttpPlan,
    port: &DetachedRuntimePublicationPort,
) -> Option<RuntimeValue> {
    let mut attempt = 0;
    loop {
        if port.is_cancelled() {
            return None;
        }
        match run_http_request(plan, port.cancellation()) {
            Ok(body) => match plan.result.success_from_text(&body) {
                Ok(value) => return Some(value),
                Err(error) => {
                    return plan
                        .result
                        .error_value(TextSourceErrorKind::Decode, &error.to_string())
                        .ok();
                }
            },
            Err(HttpRequestFailure::Cancelled) => return None,
            Err(HttpRequestFailure::TimedOut) => {
                if attempt < plan.retry_attempts {
                    attempt += 1;
                    if sleep_with_cancellation(retry_backoff(attempt), &port) {
                        return None;
                    }
                    continue;
                }
                return plan
                    .result
                    .error_value(TextSourceErrorKind::Timeout, "request timed out")
                    .ok();
            }
            Err(HttpRequestFailure::Failed(message)) => {
                if attempt < plan.retry_attempts {
                    attempt += 1;
                    if sleep_with_cancellation(retry_backoff(attempt), &port) {
                        return None;
                    }
                    continue;
                }
                return plan
                    .result
                    .error_value(TextSourceErrorKind::Request, &message)
                    .ok();
            }
        }
    }
}

fn spawn_fs_read_worker(port: DetachedRuntimePublicationPort, plan: FsReadPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) {
            return;
        }
        if sleep_with_cancellation(plan.debounce, &port) {
            return;
        }
        if stop.load(Ordering::Acquire) {
            return;
        }
        let result = match fs::read_to_string(&plan.path) {
            Ok(text) => match plan.result.success_from_text(&text) {
                Ok(value) => value,
                Err(error) => match plan
                    .result
                    .error_value(TextSourceErrorKind::Decode, &error.to_string())
                {
                    Ok(value) => value,
                    Err(_) => return,
                },
            },
            Err(error) => {
                let kind = if error.kind() == std::io::ErrorKind::NotFound {
                    TextSourceErrorKind::Missing
                } else {
                    TextSourceErrorKind::Request
                };
                match plan.result.error_value(kind, &error.to_string()) {
                    Ok(value) => value,
                    Err(_) => return,
                }
            }
        };
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(result));
    });
}

fn spawn_fs_watch_worker(port: DetachedRuntimePublicationPort, plan: FsWatchPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut previous = file_signature(&plan.path);
        while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
            thread::sleep(Duration::from_millis(40));
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                break;
            }
            let current = file_signature(&plan.path);
            let event = match (previous.exists, current.exists) {
                (false, true) => Some("Created"),
                (true, false) => Some("Deleted"),
                (true, true) if previous != current => Some("Changed"),
                _ => None,
            };
            previous = current;
            let Some(event) = event else {
                continue;
            };
            if !plan.events.contains(event) {
                continue;
            }
            let Ok(Some(value)) = plan.output.value_for_name(event) else {
                continue;
            };
            if port
                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                .is_err()
            {
                break;
            }
        }
    });
}

fn spawn_socket_worker(port: DetachedRuntimePublicationPort, plan: SocketPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            match TcpStream::connect((plan.host.as_ref(), plan.port)) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let mut reader = BufReader::with_capacity(plan.buffer.max(1), stream);
                    let mut line = String::new();
                    loop {
                        if stop.load(Ordering::Acquire) || port.is_cancelled() {
                            return;
                        }
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(_) => {
                                let line_text = line.trim_end_matches(['\r', '\n']).to_owned();
                                let value = match plan.result.success_from_text(&line_text) {
                                    Ok(value) => value,
                                    Err(error) => match plan.result.error_value(
                                        TextSourceErrorKind::Decode,
                                        &error.to_string(),
                                    ) {
                                        Ok(value) => value,
                                        Err(_) => break,
                                    },
                                };
                                if port
                                    .publish(DetachedRuntimeValue::from_runtime_owned(value))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                continue;
                            }
                            Err(error) => {
                                if let Ok(value) = plan
                                    .result
                                    .error_value(TextSourceErrorKind::Request, &error.to_string())
                                {
                                    let _ = port
                                        .publish(DetachedRuntimeValue::from_runtime_owned(value));
                                }
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Ok(value) = plan
                        .result
                        .error_value(TextSourceErrorKind::Connect, &error.to_string())
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                }
            }
            if !plan.reconnect || stop.load(Ordering::Acquire) || sleep_with_cancellation(Duration::from_millis(100), &port) {
                return;
            }
        }
    });
}

fn spawn_mailbox_worker(
    port: DetachedRuntimePublicationPort,
    plan: MailboxPlan,
    receiver: mpsc::Receiver<Box<str>>,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => {
                    let value = match plan.result.success_from_text(&message) {
                        Ok(value) => value,
                        Err(error) => match plan
                            .result
                            .error_value(TextSourceErrorKind::Decode, &error.to_string())
                        {
                            Ok(value) => value,
                            Err(_) => return,
                        },
                    };
                    if port
                        .publish(DetachedRuntimeValue::from_runtime_owned(value))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    if let Ok(value) = plan
                        .result
                        .error_value(TextSourceErrorKind::Mailbox, "mailbox disconnected")
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                    return;
                }
            }
        }
    });
}

fn spawn_process_worker(port: DetachedRuntimePublicationPort, plan: ProcessPlan, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let mut command = Command::new(plan.command.as_ref());
        command.args(plan.args.iter().map(|arg| arg.as_ref()));
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(cwd) = &plan.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &plan.env {
            command.env(key.as_ref(), value.as_ref());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(value) = plan.events.failed_value(&error.to_string()).ok().flatten() {
                    let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
                return;
            }
        };
        let pid = child.id();
        let done = Arc::new(AtomicBool::new(false));
        let cancellation = port.cancellation();
        let done_clone = done.clone();
        thread::spawn(move || {
            while !done_clone.load(Ordering::Acquire) {
                if cancellation.is_cancelled() {
                    kill_pid(pid);
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        if let Some(value) = plan.events.spawned_value().ok().flatten() {
            if port
                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                .is_err()
            {
                done.store(true, Ordering::Release);
                kill_pid(pid);
                return;
            }
        }
        let stdout_handle = child.stdout.take().map(|stdout| {
            let port = port.clone();
            let plan = plan.events.clone();
            thread::spawn(move || read_process_stream(stdout, port, plan, true))
        });
        let stderr_handle = child.stderr.take().map(|stderr| {
            let port = port.clone();
            let plan = plan.events.clone();
            thread::spawn(move || read_process_stream(stderr, port, plan, false))
        });
        let status = child.wait();
        done.store(true, Ordering::Release);
        if let Some(handle) = stdout_handle {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }
        if port.is_cancelled() {
            return;
        }
        match status {
            Ok(status) => {
                if let Some(code) = status.code() {
                    if let Some(value) = plan.events.exited_value(code as i64).ok().flatten() {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    } else if !status.success()
                        && let Some(value) = plan
                            .events
                            .failed_value(&format!("process exited with code {code}"))
                            .ok()
                            .flatten()
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                }
            }
            Err(error) => {
                if let Some(value) = plan.events.failed_value(&error.to_string()).ok().flatten() {
                    let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
            }
        }
    });
}

fn read_process_stream(
    stream: impl std::io::Read,
    port: DetachedRuntimePublicationPort,
    plan: ProcessEventPlan,
    stdout: bool,
) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while !port.is_cancelled() {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let line_text = line.trim_end_matches(['\r', '\n']);
                let value = if stdout {
                    plan.stdout_value(line_text)
                } else {
                    plan.stderr_value(line_text)
                };
                if let Ok(Some(value)) = value
                    && port
                        .publish(DetachedRuntimeValue::from_runtime_owned(value))
                        .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[derive(Debug)]
enum HttpRequestFailure {
    Cancelled,
    TimedOut,
    Failed(Box<str>),
}

fn run_http_request(
    plan: &HttpPlan,
    cancellation: CancellationObserver,
) -> Result<String, HttpRequestFailure> {
    let mut command = Command::new("curl");
    command.arg("-sS");
    command.arg("-L");
    command.arg("-X");
    command.arg(match plan.provider {
        BuiltinSourceProvider::HttpGet => "GET",
        BuiltinSourceProvider::HttpPost => "POST",
        _ => unreachable!("http plan should only be built for http providers"),
    });
    if let Some(timeout) = plan.timeout {
        command.arg("--max-time");
        command.arg(duration_seconds_string(timeout));
    }
    for (key, value) in &plan.headers {
        command.arg("-H");
        command.arg(format!("{key}: {value}"));
    }
    if let Some(body) = &plan.body {
        command.arg("--data-binary");
        command.arg(body.as_ref());
    }
    command.arg("-w");
    command.arg("\n%{http_code}");
    command.arg(plan.url.as_ref());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let child = command.spawn().map_err(|error| {
        HttpRequestFailure::Failed(format!("failed to spawn curl: {error}").into_boxed_str())
    })?;
    let pid = child.id();
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();
    let cancel_watcher = cancellation.clone();
    thread::spawn(move || {
        while !done_clone.load(Ordering::Acquire) {
            if cancel_watcher.is_cancelled() {
                kill_pid(pid);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    let output = wait_with_output(child);
    done.store(true, Ordering::Release);
    let output =
        output.map_err(|error| HttpRequestFailure::Failed(error.to_string().into_boxed_str()))?;
    if cancellation.is_cancelled() {
        return Err(HttpRequestFailure::Cancelled);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if output.status.code() == Some(28) {
            return Err(HttpRequestFailure::TimedOut);
        }
        return Err(HttpRequestFailure::Failed(stderr.into_boxed_str()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(split) = stdout.rfind('\n') else {
        return Err(HttpRequestFailure::Failed(
            "curl did not report an HTTP status code".into(),
        ));
    };
    let (body, code_text) = stdout.split_at(split);
    let status = code_text.trim().parse::<u16>().map_err(|_| {
        HttpRequestFailure::Failed("curl returned an invalid HTTP status code".into())
    })?;
    if status >= 400 {
        return Err(HttpRequestFailure::Failed(
            format!("HTTP {status}: {}", body.trim()).into_boxed_str(),
        ));
    }
    Ok(body.to_owned())
}

fn wait_with_output(child: Child) -> std::io::Result<std::process::Output> {
    child.wait_with_output()
}

fn duration_seconds_string(duration: Duration) -> String {
    format!("{}.{:03}", duration.as_secs(), duration.subsec_millis())
}

fn retry_backoff(attempt: u32) -> Duration {
    let factor = 1_u64 << attempt.min(6);
    Duration::from_millis(100_u64.saturating_mul(factor))
}

fn sleep_with_cancellation(duration: Duration, port: &DetachedRuntimePublicationPort) -> bool {
    if duration.is_zero() {
        return port.is_cancelled();
    }
    let start = Instant::now();
    while start.elapsed() < duration {
        if port.is_cancelled() {
            return true;
        }
        let remaining = duration.saturating_sub(start.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
    port.is_cancelled()
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FileSignature {
    exists: bool,
    len: u64,
    modified_millis: u128,
}

fn file_signature(path: &PathBuf) -> FileSignature {
    let Ok(metadata) = fs::metadata(path) else {
        return FileSignature::default();
    };
    let modified_millis = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    FileSignature {
        exists: true,
        len: metadata.len(),
        modified_millis,
    }
}

fn strip_signal(value: &RuntimeValue) -> &RuntimeValue {
    let mut current = value;
    while let RuntimeValue::Signal(inner) = current {
        current = inner;
    }
    current
}

fn strip_detached_signal(value: &DetachedRuntimeValue) -> &RuntimeValue {
    strip_signal(value.as_runtime())
}

fn parse_bool(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<bool, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Bool(value) => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Bool".into(),
            value: other.clone(),
        }),
    }
}

fn parse_nonnegative_int(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<i64, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "non-negative Int".into(),
            value: other.clone(),
        }),
    }
}

fn parse_text_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Text(value) => Ok(value.clone()),
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Text".into(),
            value: other.clone(),
        }),
    }
}

fn parse_text_option(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Text(value) => Ok(value.clone()),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Text".into(),
            value: other.clone(),
        }),
    }
}

fn parse_text_list(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Vec<Box<str>>, SourceProviderExecutionError> {
    let RuntimeValue::List(values) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "List Text".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    values
        .iter()
        .map(|value| match strip_signal(value) {
            RuntimeValue::Text(value) => Ok(value.clone()),
            other => Err(SourceProviderExecutionError::InvalidArgument {
                instance,
                provider,
                index,
                expected: "List Text".into(),
                value: other.clone(),
            }),
        })
        .collect()
}

fn parse_text_map(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Vec<(Box<str>, Box<str>)>, SourceProviderExecutionError> {
    let RuntimeValue::Map(entries) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Map Text Text".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    entries
        .iter()
        .map(|entry| {
            let RuntimeValue::Text(key) = strip_signal(&entry.key) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(&entry.key).clone(),
                });
            };
            let RuntimeValue::Text(value) = strip_signal(&entry.value) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(&entry.value).clone(),
                });
            };
            Ok((key.clone(), value.clone()))
        })
        .collect()
}

fn parse_named_variants(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<BTreeSet<Box<str>>, SourceProviderExecutionError> {
    let RuntimeValue::List(values) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "List payloadless variants".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    values
        .iter()
        .map(|value| {
            variant_name_value(strip_signal(value)).ok_or_else(|| {
                SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "List payloadless variants".into(),
                    value: strip_signal(value).clone(),
                }
            })
        })
        .collect()
}

fn parse_duration(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Duration, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount =
                raw.parse::<u64>()
                    .map_err(|_| SourceProviderExecutionError::InvalidArgument {
                        instance,
                        provider,
                        index,
                        expected: "Duration".into(),
                        value: value.to_runtime(),
                    })?;
            duration_from_suffix(amount, suffix).ok_or_else(|| {
                SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Duration".into(),
                    value: value.to_runtime(),
                }
            })
        }
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Duration".into(),
            value: other.clone(),
        }),
    }
}

fn parse_option_duration(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Duration, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(duration_ms) if *duration_ms <= 0 => {
            Err(SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "positive Duration".into(),
                value: RuntimeValue::Int(*duration_ms),
            })
        }
        RuntimeValue::Int(value) if *value >= 0 => Ok(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount =
                raw.parse::<u64>()
                    .map_err(|_| SourceProviderExecutionError::InvalidOption {
                        instance,
                        provider,
                        option_name: option_name.into(),
                        expected: "Duration".into(),
                        value: value.to_runtime(),
                    })?;
            duration_from_suffix(amount, suffix).ok_or_else(|| {
                SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Duration".into(),
                    value: value.to_runtime(),
                }
            })
        }
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Duration".into(),
            value: other.clone(),
        }),
    }
}

fn parse_retry(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<u32, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(*value as u32),
        RuntimeValue::SuffixedInteger { raw, suffix } if suffix.as_ref() == "x" => raw
            .parse::<u32>()
            .map_err(|_| SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "Retry".into(),
                value: value.to_runtime(),
            }),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Retry".into(),
            value: other.clone(),
        }),
    }
}

fn parse_stream_mode(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<ProcessStreamMode, SourceProviderExecutionError> {
    let value = strip_detached_signal(value);
    let Some(name) = variant_name_value(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "StreamMode".into(),
            value: value.clone(),
        });
    };
    match name.as_ref() {
        "Ignore" => Ok(ProcessStreamMode::Ignore),
        "Lines" => Ok(ProcessStreamMode::Lines),
        "Bytes" => Err(SourceProviderExecutionError::UnsupportedOption {
            instance,
            provider,
            option_name: option_name.into(),
        }),
        _ => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "StreamMode".into(),
            value: value.clone(),
        }),
    }
}

fn variant_name_value(value: &RuntimeValue) -> Option<Box<str>> {
    match value {
        RuntimeValue::Sum(value) if value.fields.is_empty() => Some(value.variant_name.clone()),
        RuntimeValue::Text(value) => Some(value.clone()),
        RuntimeValue::Callable(RuntimeCallable::SumConstructor {
            handle,
            bound_arguments,
        }) if handle.field_count == 0 && bound_arguments.is_empty() => {
            Some(handle.variant_name.clone())
        }
        _ => None,
    }
}

fn encode_runtime_body(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    let value = strip_detached_signal(value);
    match value {
        RuntimeValue::Text(value) => Ok(value.clone()),
        _ => encode_runtime_json(value)
            .map_err(
                |detail| SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: format!("http body encoding failed: {detail}").into_boxed_str(),
                },
            )
            .map(String::into_boxed_str),
    }
}

fn duration_from_suffix(amount: u64, suffix: &str) -> Option<Duration> {
    match suffix {
        "ns" => Some(Duration::from_nanos(amount)),
        "us" => Some(Duration::from_micros(amount)),
        "ms" => Some(Duration::from_millis(amount)),
        "s" => Some(Duration::from_secs(amount)),
        "m" => amount.checked_mul(60).map(Duration::from_secs),
        "h" => amount.checked_mul(60 * 60).map(Duration::from_secs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module as lower_hir_module};
    use aivi_lambda::lower_module as lower_lambda_module;
    use aivi_syntax::parse_module;

    use super::*;
    use crate::{BackendLinkedRuntime, assemble_hir_runtime, link_backend_runtime};

    struct LoweredStack {
        hir: aivi_hir::LoweringResult,
        core: aivi_core::Module,
        backend: aivi_backend::Program,
    }

    fn lower_text(path: &str, text: &str) -> LoweredStack {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let hir = lower_hir_module(&parsed.module);
        assert!(
            !hir.has_errors(),
            "fixture {path} should lower to HIR: {:?}",
            hir.diagnostics()
        );
        let core =
            aivi_core::lower_module(hir.module()).expect("typed-core lowering should succeed");
        let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
        let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
        LoweredStack { hir, core, backend }
    }

    fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Value(item) if item.name.text() == name => Some(item_id),
                Item::Function(item) if item.name.text() == name => Some(item_id),
                Item::Signal(item) if item.name.text() == name => Some(item_id),
                Item::Type(item) if item.name.text() == name => Some(item_id),
                Item::Class(item) if item.name.text() == name => Some(item_id),
                Item::Domain(item) if item.name.text() == name => Some(item_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    fn spin_until(
        linked: &mut BackendLinkedRuntime,
        signal: crate::SignalHandle,
        timeout: Duration,
    ) -> Option<RuntimeValue> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            linked.tick().expect("runtime tick should succeed");
            if let Some(value) = linked.runtime().current_value(signal).unwrap() {
                return Some(value.clone());
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn run_http_server(response_body: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept one request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });
        (format!("http://{address}"), handle)
    }

    fn run_http_server_sequence(responses: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for response_body in responses {
                let (mut stream, _) = listener.accept().expect("server should accept a request");
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("server should write response");
            }
        });
        (format!("http://{address}"), handle)
    }

    fn temp_path(prefix: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        env::temp_dir().join(format!(
            "aivi-runtime-{prefix}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn timer_every_actions_publish_unit_immediately() {
        let lowered = lower_text(
            "runtime-provider-timer-every.aivi",
            r#"
@source timer.every 5 with {
    immediate: True
}
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");

        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("timer provider actions should execute");

        let tick_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "tick"))
            .expect("tick signal binding should exist")
            .signal();
        assert_eq!(
            spin_until(&mut linked, tick_signal, Duration::from_millis(200)),
            Some(RuntimeValue::Unit)
        );
    }

    #[test]
    fn http_get_publishes_decoded_result_values() {
        let (base_url, handle) = run_http_server(r#"[{"id":1,"name":"Ada"}]"#);
        let lowered = lower_text(
            "runtime-provider-http-get.aivi",
            &format!(
                r#"
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {{
    id: Int,
    name: Text
}}

@source http.get "{base_url}/users"
sig users : Signal (Result HttpError (List User))
"#
            ),
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("http provider should execute");
        let users_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "users"))
            .expect("users signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, users_signal, Duration::from_secs(2))
            .expect("http provider should publish a result");
        assert!(matches!(value, RuntimeValue::ResultOk(_)));
        handle.join().unwrap();
    }

    #[test]
    fn http_get_refresh_every_reissues_requests() {
        let (base_url, handle) = run_http_server_sequence(vec!["first", "second"]);
        let lowered = lower_text(
            "runtime-provider-http-refresh.aivi",
            &format!(
                r#"
type HttpError =
  | Timeout
  | DecodeFailure Text
  | RequestFailure Text

@source http.get "{base_url}/users" with {{
    refreshEvery: 40
}}
sig users : Signal (Result HttpError Text)
"#
            ),
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("http provider should execute");
        let users_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "users"))
            .expect("users signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, users_signal, Duration::from_secs(1))
            .expect("http provider should refresh and publish");
        if value != RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into()))) {
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut latest = value;
            while Instant::now() < deadline {
                linked.tick().expect("runtime tick should succeed");
                if let Some(current) = linked.runtime().current_value(users_signal).unwrap() {
                    latest = current.clone();
                    if latest
                        == RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into())))
                    {
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                latest,
                RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into())))
            );
        }
        handle.join().unwrap();
    }

    #[test]
    fn mailbox_source_publishes_text_messages() {
        let lowered = lower_text(
            "runtime-provider-mailbox.aivi",
            r#"
type MailboxError =
  | DecodeFailure Text
  | MailboxFailure Text

@source mailbox.subscribe "jobs"
sig job : Signal (Result MailboxError Text)
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("mailbox source should execute");
        providers
            .publish_mailbox_message("jobs", "hello")
            .expect("mailbox publish should succeed");
        let job_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "job"))
            .expect("job signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, job_signal, Duration::from_millis(200))
            .expect("mailbox source should publish");
        assert_eq!(
            value,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
        );
    }

    #[test]
    fn mailbox_source_unsubscribes_on_suspension() {
        let lowered = lower_text(
            "runtime-provider-mailbox-suspend.aivi",
            r#"
type MailboxError =
  | DecodeFailure Text
  | MailboxFailure Text

@source mailbox.subscribe "jobs"
sig job : Signal (Result MailboxError Text)
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("mailbox source should execute");
        let job_item = item_id(lowered.hir.module(), "job");
        let instance = linked
            .source_by_owner(job_item)
            .expect("job source binding should exist")
            .instance;
        linked
            .runtime_mut()
            .suspend_source(instance)
            .expect("source suspension should succeed");
        providers
            .apply_actions(&[crate::LinkedSourceLifecycleAction::Suspend { instance }])
            .expect("provider suspension should succeed");
        providers
            .publish_mailbox_message("jobs", "later")
            .expect("mailbox publish should still succeed");
        let signal = linked
            .assembly()
            .signal(job_item)
            .expect("job signal binding should exist")
            .signal();
        assert!(
            spin_until(&mut linked, signal, Duration::from_millis(150)).is_none(),
            "suspended mailbox sources should stop receiving messages"
        );
    }

    #[test]
    fn fs_read_publishes_text_snapshots() {
        let path = temp_path("fs-read");
        fs::write(&path, "hello").expect("fixture file should write");
        let lowered = lower_text(
            "runtime-provider-fs-read.aivi",
            &format!(
                r#"
type FsError =
  | Missing
  | DecodeFailure Text

@source fs.read "{}"
sig fileText : Signal (Result FsError Text)
"#,
                path.display()
            ),
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("fs.read source should execute");
        let signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "fileText"))
            .expect("fileText signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, signal, Duration::from_millis(300))
            .expect("fs.read should publish one snapshot");
        assert_eq!(
            value,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn fs_watch_detects_created_files() {
        let path = temp_path("fs-watch");
        let lowered = lower_text(
            "runtime-provider-fs-watch.aivi",
            &format!(
                r#"
type FsWatchEvent =
  | Created
  | Changed
  | Deleted

@source fs.watch "{}"
sig fileEvents : Signal FsWatchEvent
"#,
                path.display()
            ),
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("fs.watch source should execute");
        thread::sleep(Duration::from_millis(100));
        fs::write(&path, "hello").expect("watched file should write");
        let signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "fileEvents"))
            .expect("fileEvents signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, signal, Duration::from_secs(1))
            .expect("fs.watch should publish a create event");
        assert!(matches!(value, RuntimeValue::Sum(_)));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn socket_connect_reads_text_lines() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept one client");
            stream
                .write_all(b"hello\n")
                .expect("server should write one line");
        });
        let lowered = lower_text(
            "runtime-provider-socket.aivi",
            &format!(
                r#"
type SocketError =
  | ConnectFailure Text
  | DecodeFailure Text
  | RequestFailure Text

@source socket.connect "tcp://{}:{}"
sig message : Signal (Result SocketError Text)
"#,
                address.ip(),
                address.port()
            ),
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("socket source should execute");
        let signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "message"))
            .expect("message signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, signal, Duration::from_secs(1))
            .expect("socket source should publish one line");
        assert_eq!(
            value,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
        );
        handle.join().unwrap();
    }

    #[test]
    fn process_spawn_publishes_process_events() {
        let lowered = lower_text(
            "runtime-provider-process.aivi",
            r#"
type StreamMode =
  | Ignore
  | Lines

type ProcessEvent =
  | Spawned

@source process.spawn "true"
sig events : Signal ProcessEvent
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("process source should execute");
        let signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "events"))
            .expect("events signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, signal, Duration::from_secs(1))
            .expect("process source should publish at least one event");
        assert!(matches!(value, RuntimeValue::Sum(_)));
    }

    #[test]
    fn window_key_source_suppresses_repeat_when_requested() {
        let lowered = lower_text(
            "runtime-provider-window-key.aivi",
            r#"
type Key =
  | ArrowUp
  | ArrowDown

@source window.keyDown with {
    repeat: False
    focusOnly: True
}
sig keyDown : Signal Key
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("window key source should execute");
        providers.dispatch_window_key_event(WindowKeyEvent {
            name: "ArrowUp".into(),
            repeated: false,
        });
        providers.dispatch_window_key_event(WindowKeyEvent {
            name: "ArrowUp".into(),
            repeated: true,
        });
        let key_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "keyDown"))
            .expect("keyDown signal binding should exist")
            .signal();
        let value = spin_until(&mut linked, key_signal, Duration::from_millis(200))
            .expect("window key source should publish");
        assert!(matches!(value, RuntimeValue::Sum(_)));
    }

    #[test]
    fn timer_every_stops_after_source_suspension() {
        let lowered = lower_text(
            "runtime-provider-timer-cancel.aivi",
            r#"
@source timer.every 5
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");

        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("timer provider actions should execute");

        let tick_item = item_id(lowered.hir.module(), "tick");
        let instance = linked
            .source_by_owner(tick_item)
            .expect("tick source binding should exist")
            .instance;
        let tick_signal = linked
            .assembly()
            .signal(tick_item)
            .expect("tick signal binding should exist")
            .signal();

        assert!(spin_until(&mut linked, tick_signal, Duration::from_millis(200)).is_some());

        linked
            .runtime_mut()
            .suspend_source(instance)
            .expect("source suspension should cancel the active timer port");
        providers
            .apply_actions(&[crate::LinkedSourceLifecycleAction::Suspend { instance }])
            .expect("provider manager should drop suspended timer state");

        let quiet_deadline = Instant::now() + Duration::from_millis(200);
        loop {
            thread::sleep(Duration::from_millis(12));
            let outcome = linked
                .tick()
                .expect("runtime tick should stay quiet after timer cancellation");
            assert!(
                outcome.committed().is_empty(),
                "suspended timers should not commit further values"
            );
            if outcome.dropped_publications().is_empty() {
                break;
            }
            assert!(
                Instant::now() < quiet_deadline,
                "suspended timers should stop publishing after draining any in-flight delivery"
            );
        }

        for _ in 0..5 {
            thread::sleep(Duration::from_millis(12));
            let outcome = linked
                .tick()
                .expect("runtime tick should stay quiet after timer cancellation");
            assert!(
                outcome.committed().is_empty(),
                "suspended timers should not commit further values"
            );
            assert!(
                outcome.dropped_publications().is_empty(),
                "suspended timers should stop publishing instead of producing stale drops"
            );
        }
    }
}
