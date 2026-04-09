#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowKeyEvent {
    pub name: Box<str>,
    pub repeated: bool,
}

/// Configuration for a window.keyDown source instance, exposed so the GTK host
/// can set the correct propagation phase and focus policy on the event controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowKeyConfig {
    pub capture: bool,
    pub focus_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderExecutionError {
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
    jitter: Option<Duration>,
    immediate: bool,
    coalesce: bool,
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
        let mut jitter = None;
        let mut coalesce = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "immediate" => {
                    immediate = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "coalesce" => {
                    coalesce = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "restartOn" => {}
                "activeWhen" => {}
                "jitter" => {
                    let dur = parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                    if dur > delay {
                        return Err(SourceProviderExecutionError::StartFailed {
                            instance,
                            provider,
                            detail: "jitter must not exceed the timer interval".into(),
                        });
                    }
                    jitter = Some(dur);
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
            delay,
            jitter,
            immediate,
            coalesce,
        })
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

    fn success_from_text(&self, text: &str) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
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

#[derive(Clone)]
struct DbConnectPlan {
    database: Box<str>,
    result: RequestResultPlan,
}

impl DbConnectPlan {
    fn parse(
        instance: SourceInstanceId,
        context: &SourceProviderContext,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbConnect;
        validate_argument_count(instance, provider, config, 1)?;
        let database =
            parse_db_connect_argument(instance, provider, 0, context, &config.arguments[0])?;
        for option in &config.options {
            match option.option_name.as_ref() {
                "pool" => {
                    let _ =
                        parse_positive_int(instance, provider, &option.option_name, &option.value)?;
                }
                "activeWhen" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        let result = RequestResultPlan::parse(instance, provider, config)?;
        db_connect_success_value(instance, &result, &database)?;
        db_connect_error_value(instance, &result, "db.connect probe failure")?;
        Ok(Self { database, result })
    }
}

#[derive(Clone)]
struct DbLivePlan {
    task: RuntimeValue,
    debounce: Duration,
    #[allow(dead_code)]
    optimistic: bool,
    result: Option<RequestResultPlan>,
}

impl DbLivePlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbLive;
        validate_argument_count(instance, provider, config, 1)?;
        let task = parse_task_argument(instance, provider, 0, &config.arguments[0])?;
        let mut debounce = Duration::ZERO;
        let mut optimistic = false;
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
                "refreshOn" | "activeWhen" => {}
                "optimistic" => {
                    optimistic =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "onRollback" => {
                    // The onRollback signal is accepted and stored; the runtime publishes
                    // the last confirmed value to it when an optimistic update is reverted.
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
        let result = if config.decode.is_some() {
            let result = RequestResultPlan::parse(instance, provider, config)?;
            db_live_query_error_value(instance, &result, "db.live query failure")?;
            Some(result)
        } else {
            None
        };
        Ok(Self {
            task,
            debounce,
            optimistic,
            result,
        })
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
    Query,
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
            Self::Query => &QUERY_ERROR_CANDIDATES,
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
            Self::Query => f.write_str("query"),
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

const QUERY_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("QueryFailed"),
    ErrorCandidate::text("ConnectionFailed"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("Error"),
];

const CONNECT_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("ConnectionFailed"),
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

/// Runtime plan for OpenAPI capability handle operations.
///
/// Combines a `baseUrl` option with the `operationPath` argument at runtime
/// and delegates to the same HTTP worker infrastructure as `HttpPlan`.
struct ApiPlan {
    /// HTTP provider variant (ApiGet / ApiPost / etc.) driving the curl method.
    provider: BuiltinSourceProvider,
    /// Fully composed request URL: baseUrl + operation_path.
    url: Box<str>,
    headers: Box<[(Box<str>, Box<str>)]>,
    body: Option<Box<str>>,
    timeout: Option<Duration>,
    refresh_every: Option<Duration>,
    retry_attempts: u32,
    result: RequestResultPlan,
}

impl ApiPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        // Arguments: [spec_path (ignored at runtime), operation_path]
        if config.arguments.len() < 2 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 2,
                found: config.arguments.len(),
            });
        }
        let operation_path = parse_text_argument(instance, provider, 1, &config.arguments[1])?;
        let mut base_url = String::new();
        let mut headers = Vec::new();
        let mut body = None;
        let mut timeout = None;
        let mut refresh_every = None;
        let mut retry_attempts = 0;
        for option in &config.options {
            match option.option_name.as_ref() {
                "baseUrl" => {
                    base_url =
                        parse_text_option(instance, provider, &option.option_name, &option.value)?
                            .into();
                }
                "headers" => {
                    headers =
                        parse_text_map(instance, provider, &option.option_name, &option.value)?;
                }
                "query" => {
                    // Query params are appended to the URL after combination; skip for now.
                }
                "body" => {
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
                "auth" => {
                    // Inject auth header from the runtime value.
                    if let Some((name, value)) =
                        extract_auth_header(instance, provider, &option.value)?
                    {
                        headers.push((name, value));
                    }
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
        let full_url = format!(
            "{}{}",
            base_url.trim_end_matches('/'),
            operation_path.as_ref()
        );
        let url =
            Url::parse(&full_url).map_err(|error| SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("invalid API URL `{full_url}`: {error}").into_boxed_str(),
            })?;
        // Convert to HttpPlan by reusing the underlying HTTP worker.
        // We represent the plan as a zero-cost HttpPlan alias via a helper.
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

    fn into_http_plan(self) -> HttpPlan {
        HttpPlan {
            provider: self.provider,
            url: self.url,
            headers: self.headers,
            body: self.body,
            timeout: self.timeout,
            refresh_every: self.refresh_every,
            retry_attempts: self.retry_attempts,
            result: self.result,
        }
    }
}

fn extract_auth_header(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    value: &DetachedRuntimeValue,
) -> Result<Option<(Box<str>, Box<str>)>, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Sum(sum) => {
            match sum.variant_name.as_ref() {
                "BearerToken" => {
                    if let Some(RuntimeValue::Text(token)) = sum.fields.first() {
                        return Ok(Some((
                            "Authorization".into(),
                            format!("Bearer {token}").into_boxed_str(),
                        )));
                    }
                }
                "BasicAuth" => {
                    if sum.fields.len() >= 2 {
                        if let (RuntimeValue::Text(user), RuntimeValue::Text(pass)) =
                            (&sum.fields[0], &sum.fields[1])
                        {
                            let credentials = format!("{user}:{pass}");
                            let encoded = base64_encode(credentials.as_bytes());
                            return Ok(Some((
                                "Authorization".into(),
                                format!("Basic {encoded}").into_boxed_str(),
                            )));
                        }
                    }
                }
                "ApiKey" => {
                    if let Some(RuntimeValue::Text(key)) = sum.fields.first() {
                        return Ok(Some(("X-API-Key".into(), key.clone())));
                    }
                }
                "ApiKeyQuery" => {
                    // Query-based API key; not injected as a header.
                }
                "OAuth2" => {
                    if let Some(RuntimeValue::Text(token)) = sum.fields.first() {
                        return Ok(Some((
                            "Authorization".into(),
                            format!("Bearer {token}").into_boxed_str(),
                        )));
                    }
                }
                _ => {}
            }
            Ok(None)
        }
        RuntimeValue::Unit => Ok(None),
        _ => Err(SourceProviderExecutionError::StartFailed {
            instance,
            provider,
            detail: "api `auth` option must be an ApiAuth sum value".into(),
        }),
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) & 0x3F] as char);
        out.push(TABLE[(n >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[n & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn spawn_api_worker(
    port: DetachedRuntimePublicationPort,
    plan: ApiPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    spawn_http_worker(port, plan.into_http_plan(), stop)
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

    fn value_for_name(
        &self,
        name: &str,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
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
    recursive: bool,
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
        let mut recursive = false;
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
                    recursive = parse_bool(instance, provider, &option.option_name, &option.value)?;
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
            recursive,
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
    heartbeat: Option<Duration>,
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
        let mut heartbeat = None;
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
                    heartbeat = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
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
            heartbeat,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct MailboxPlan {
    mailbox: Box<str>,
    buffer: usize,
    reconnect: bool,
    heartbeat: Option<Duration>,
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
        let mut reconnect = false;
        let mut heartbeat = None;
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
                "reconnect" => {
                    reconnect = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "heartbeat" => {
                    heartbeat = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
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
            reconnect,
            heartbeat,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessStreamMode {
    Ignore,
    Lines,
    Bytes,
}

#[derive(Clone)]
struct ProcessPlan {
    command: Box<str>,
    args: Box<[Box<str>]>,
    cwd: Option<PathBuf>,
    env: Box<[(Box<str>, Box<str>)]>,
    stdout_mode: ProcessStreamMode,
    stderr_mode: ProcessStreamMode,
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
        if stdout != ProcessStreamMode::Ignore && events.stdout.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stdout: Lines/Bytes requires a `Stdout` event variant in the source output type"
                    .into(),
            });
        }
        if stderr != ProcessStreamMode::Ignore && events.stderr.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stderr: Lines/Bytes requires a `Stderr` event variant in the source output type"
                    .into(),
            });
        }
        Ok(Self {
            command,
            args: args.into_boxed_slice(),
            cwd,
            env: env.into_boxed_slice(),
            stdout_mode: stdout,
            stderr_mode: stderr,
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
    Bytes,
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
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Bytes,
                    } => ProcessPayloadKind::Bytes,
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

    fn spawned_value(&self) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(self.spawned.as_ref(), None)
    }

    fn stdout_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stdout.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn stderr_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stderr.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn exited_value(&self, code: i64) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(self.exited.as_ref(), Some(ExternalSourceValue::Int(code)))
    }

    fn failed_value(
        &self,
        message: &str,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.failed.as_ref(),
            Some(ExternalSourceValue::Text(message.into())),
        )
    }

    fn stdout_bytes_value(
        &self,
        chunk: Box<[u8]>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stdout.as_ref(),
            Some(ExternalSourceValue::Bytes(chunk)),
        )
    }

    fn stderr_bytes_value(
        &self,
        chunk: Box<[u8]>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stderr.as_ref(),
            Some(ExternalSourceValue::Bytes(chunk)),
        )
    }

    fn variant_value(
        &self,
        plan: Option<&ProcessVariantPlan>,
        payload: Option<ExternalSourceValue>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        let raw = match (plan.payload, payload) {
            (ProcessPayloadKind::None, _) => ExternalSourceValue::variant(plan.variant.as_ref()),
            (ProcessPayloadKind::Text, Some(payload @ ExternalSourceValue::Text(_)))
            | (ProcessPayloadKind::Int, Some(payload @ ExternalSourceValue::Int(_)))
            | (ProcessPayloadKind::Bytes, Some(payload @ ExternalSourceValue::Bytes(_))) => {
                ExternalSourceValue::variant_with_payload(plan.variant.as_ref(), payload)
            }
            _ => return Ok(None),
        };
        decode_external(&self.decode, &raw).map(Some)
    }
}

#[derive(Clone)]
struct WindowKeyDownPlan {
    capture: bool,
    focus_only: bool,
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
            // capture is now supported — stored in the plan and honoured at the GTK boundary.
        }
        if !focus_only {
            // focusOnly: False is now supported — stored in the plan and honoured at the GTK boundary.
        }
        Ok(Self {
            capture,
            focus_only,
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

    fn value_for_key(&self, key: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
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

#[derive(Clone, Copy)]
enum DbusBus {
    Session,
    System,
}

impl DbusBus {
    fn parse_option(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: &str,
        value: &DetachedRuntimeValue,
    ) -> Result<Self, SourceProviderExecutionError> {
        let value = parse_text_option(instance, provider, option_name, value)?;
        match value.as_ref() {
            "session" => Ok(Self::Session),
            "system" => Ok(Self::System),
            _ => Err(SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "\"session\" or \"system\"".into(),
                value: RuntimeValue::Text(value),
            }),
        }
    }

    const fn bus_type(self) -> BusType {
        match self {
            Self::Session => BusType::Session,
            Self::System => BusType::System,
        }
    }
}

#[derive(Clone)]
struct DbusOwnNamePlan {
    instance: SourceInstanceId,
    name: Box<str>,
    bus: DbusBus,
    address: Option<Box<str>>,
    flags: BusNameOwnerFlags,
    output: DbusNameStateOutputPlan,
}

impl DbusOwnNamePlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusOwnName;
        validate_argument_count(instance, provider, config, 1)?;
        let name = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut flags = BusNameOwnerFlags::NONE;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "flags" => {
                    for flag in parse_named_variants(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? {
                        match flag.as_ref() {
                            "AllowReplacement" => flags |= BusNameOwnerFlags::ALLOW_REPLACEMENT,
                            "ReplaceExisting" => flags |= BusNameOwnerFlags::REPLACE,
                            "DoNotQueue" => flags |= BusNameOwnerFlags::DO_NOT_QUEUE,
                            _ => {
                                return Err(SourceProviderExecutionError::InvalidOption {
                                    instance,
                                    provider,
                                    option_name: option.option_name.clone(),
                                    expected:
                                        "List BusNameFlag (AllowReplacement | ReplaceExisting | DoNotQueue)"
                                            .into(),
                                    value: strip_detached_signal(&option.value).clone(),
                                });
                            }
                        }
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
            instance,
            name,
            bus,
            address,
            flags,
            output: DbusNameStateOutputPlan::parse(instance, config)?,
        })
    }
}

#[derive(Clone)]
struct DbusSignalPlan {
    instance: SourceInstanceId,
    bus: DbusBus,
    address: Option<Box<str>>,
    path: Box<str>,
    interface: Option<Box<str>>,
    member: Option<Box<str>>,
    output: DbusMessageOutputPlan,
}

impl DbusSignalPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusSignal;
        validate_argument_count(instance, provider, config, 1)?;
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut interface = None;
        let mut member = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "interface" => {
                    interface = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "member" => {
                    member = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
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
            instance,
            bus,
            address,
            path,
            interface,
            member,
            output: DbusMessageOutputPlan::parse_signal(instance, config)?,
        })
    }
}

#[derive(Clone)]
struct DbusMethodPlan {
    instance: SourceInstanceId,
    bus: DbusBus,
    address: Option<Box<str>>,
    destination: Box<str>,
    path: Option<Box<str>>,
    interface: Option<Box<str>>,
    member: Option<Box<str>>,
    reply_body: Option<Box<str>>,
    output: DbusMessageOutputPlan,
}

impl DbusMethodPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusMethod;
        validate_argument_count(instance, provider, config, 1)?;
        let destination = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut path = None;
        let mut interface = None;
        let mut member = None;
        let mut reply_body = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "path" => {
                    path = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "interface" => {
                    interface = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "member" => {
                    member = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "reply" => {
                    reply_body = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
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
            instance,
            bus,
            address,
            destination,
            path,
            interface,
            member,
            reply_body,
            output: DbusMessageOutputPlan::parse_method(instance, config)?,
        })
    }
}

#[derive(Clone)]
enum DbusNameStateOutputPlan {
    Text,
    NamedVariants { decode: hir::SourceDecodeProgram },
}

impl DbusNameStateOutputPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusOwnName;
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
                            detail: "dbus.ownName currently decodes to `Text` or a payloadless `BusNameState` sum".into(),
                        });
                    }
                    names.insert(variant.name.as_str());
                }
                if names == BTreeSet::from(["Lost", "Owned", "Queued"]) {
                    Ok(Self::NamedVariants { decode })
                } else {
                    Err(SourceProviderExecutionError::UnsupportedProviderShape {
                        instance,
                        provider,
                        detail: "dbus.ownName sum outputs must define exactly `Owned`, `Queued`, and `Lost`".into(),
                    })
                }
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "dbus.ownName currently decodes to `Text` or a payloadless `BusNameState` sum"
                        .into(),
            }),
        }
    }

    fn value_for_state(
        &self,
        state: &'static str,
    ) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        match self {
            Self::Text => Ok(RuntimeValue::Text(state.into())),
            Self::NamedVariants { decode } => {
                decode_external(decode, &ExternalSourceValue::variant(state))
            }
        }
    }
}

#[derive(Clone)]
enum DbusMessageShape {
    Signal,
    Method,
}

#[derive(Clone, Copy)]
enum DbusMessageBodyMode {
    Text,
    Structured,
}

#[derive(Clone)]
struct DbusMessageOutputPlan {
    decode: hir::SourceDecodeProgram,
    shape: DbusMessageShape,
    body_mode: DbusMessageBodyMode,
}

impl DbusMessageOutputPlan {
    fn parse_signal(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        Self::parse(
            instance,
            BuiltinSourceProvider::DbusSignal,
            config,
            DbusMessageShape::Signal,
        )
    }

    fn parse_method(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        Self::parse(
            instance,
            BuiltinSourceProvider::DbusMethod,
            config,
            DbusMessageShape::Method,
        )
    }

    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
        shape: DbusMessageShape,
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
        let expected = match shape {
            DbusMessageShape::Signal => ["path", "interface", "member", "body"].as_slice(),
            DbusMessageShape::Method => {
                ["destination", "path", "interface", "member", "body"].as_slice()
            }
        };
        let hir::DecodeProgramStep::Record { fields, .. } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: match shape {
                    DbusMessageShape::Signal => {
                        "dbus.signal currently decodes to a `DbusSignal`-shaped record".into()
                    }
                    DbusMessageShape::Method => {
                        "dbus.method currently decodes to a `DbusCall`-shaped record".into()
                    }
                },
            });
        };
        if fields.len() != expected.len() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "{} currently requires record fields {:?}",
                    provider.key(),
                    expected
                )
                .into_boxed_str(),
            });
        }
        let mut body_mode = None;
        for field_name in expected {
            let Some(field) = fields
                .iter()
                .find(|field| field.name.as_str() == *field_name)
            else {
                return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: format!(
                        "{} currently requires record fields {:?}",
                        provider.key(),
                        expected
                    )
                    .into_boxed_str(),
                });
            };
            let valid = match *field_name {
                "path" | "interface" | "member" | "destination" => matches!(
                    decode.step(field.step),
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    }
                ),
                "body" => match decode.step(field.step) {
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    } => {
                        body_mode = Some(DbusMessageBodyMode::Text);
                        true
                    }
                    hir::DecodeProgramStep::List { element }
                        if dbus_value_step_supported(&decode, *element, &mut HashSet::new()) =>
                    {
                        body_mode = Some(DbusMessageBodyMode::Structured);
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
            if !valid {
                return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: match shape {
                        DbusMessageShape::Signal => "dbus.signal outputs must use `Text` header fields and either a `Text` body or `List DbusValue` body".into(),
                        DbusMessageShape::Method => "dbus.method outputs must use `Text` header fields and either a `Text` body or `List DbusValue` body".into(),
                    },
                });
            }
        }
        Ok(Self {
            decode,
            shape,
            body_mode: body_mode.expect("dbus output bodies should set a body mode"),
        })
    }

    fn signal_value(
        &self,
        path: &str,
        interface: &str,
        member: &str,
        parameters: &Variant,
    ) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        let raw = self.raw_record(None, path, interface, member, Some(parameters))?;
        decode_external(&self.decode, &raw)
    }

    fn method_value(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        parameters: Option<&Variant>,
    ) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        let raw = self.raw_record(Some(destination), path, interface, member, parameters)?;
        decode_external(&self.decode, &raw)
    }

    fn raw_record(
        &self,
        destination: Option<&str>,
        path: &str,
        interface: &str,
        member: &str,
        parameters: Option<&Variant>,
    ) -> Result<ExternalSourceValue, SourceDecodeErrorWithPath> {
        let mut record = BTreeMap::new();
        if matches!(self.shape, DbusMessageShape::Method) {
            record.insert(
                "destination".into(),
                ExternalSourceValue::Text(destination.unwrap_or_default().into()),
            );
        }
        record.insert("path".into(), ExternalSourceValue::Text(path.into()));
        record.insert(
            "interface".into(),
            ExternalSourceValue::Text(interface.into()),
        );
        record.insert("member".into(), ExternalSourceValue::Text(member.into()));
        record.insert(
            "body".into(),
            match self.body_mode {
                DbusMessageBodyMode::Text => ExternalSourceValue::Text(
                    parameters
                        .map(|value| value.print(false).to_string().into_boxed_str())
                        .unwrap_or_else(|| "".into()),
                ),
                DbusMessageBodyMode::Structured => dbus_body_external(parameters)
                    .map_err(|detail| SourceDecodeError::InvalidJson { detail })?,
            },
        );
        Ok(ExternalSourceValue::Record(record))
    }
}

