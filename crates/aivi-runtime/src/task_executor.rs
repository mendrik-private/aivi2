use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aivi_backend::{
    ItemId, RuntimeCustomCapabilityCommandPlan, RuntimeDbCommitPlan, RuntimeDbQueryPlan,
    RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeFloat, RuntimeMap, RuntimeMapEntry,
    RuntimeTaskPlan, RuntimeValue, TaskFunctionApplier,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gio::DBusCallFlags;
use glib::{Variant, VariantTy, prelude::ToVariant};
use regex::Regex;
use secret_service::{EncryptionType, blocking::SecretService};
use sha2::{Digest, Sha256};
use url::Url;

use crate::providers::{
    SourceProviderContext, open_dbus_connection_text, runtime_dbus_body_from_variant,
    runtime_dbus_values_to_variant,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTaskExecutionError {
    message: Box<str>,
}

impl RuntimeTaskExecutionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into_boxed_str(),
        }
    }
}

impl fmt::Display for RuntimeTaskExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeTaskExecutionError {}

pub trait CustomCapabilityCommandExecutor: Send + Sync {
    fn execute(
        &self,
        context: &SourceProviderContext,
        plan: &RuntimeCustomCapabilityCommandPlan,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> Result<RuntimeValue, RuntimeTaskExecutionError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeDbCommitInvalidation {
    pub connection: aivi_backend::RuntimeDbConnection,
    pub changed_tables: BTreeSet<Box<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeTaskExecutionOutcome {
    pub value: RuntimeValue,
    pub commit_invalidation: Option<RuntimeDbCommitInvalidation>,
}

impl RuntimeTaskExecutionOutcome {
    fn value(value: RuntimeValue) -> Self {
        Self {
            value,
            commit_invalidation: None,
        }
    }
}

pub fn execute_runtime_task_plan(
    plan: RuntimeTaskPlan,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    execute_runtime_task_plan_with_context(plan, &SourceProviderContext::current(), stdout, stderr)
}

pub fn execute_runtime_task_plan_with_context(
    plan: RuntimeTaskPlan,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    match plan {
        RuntimeTaskPlan::Pure { value } => Ok(*value),
        RuntimeTaskPlan::RandomInt { low, high } => {
            Ok(RuntimeValue::Int(sample_random_i64_inclusive(low, high)?))
        }
        RuntimeTaskPlan::RandomBytes { count } => {
            let count = usize::try_from(count).map_err(|_| {
                task_error(format!(
                    "random byte count must be non-negative, found {count}"
                ))
            })?;
            Ok(RuntimeValue::Bytes(read_os_random_bytes(count)?))
        }
        RuntimeTaskPlan::StdoutWrite { text } => {
            stdout
                .write_all(text.as_bytes())
                .map_err(|error| task_error(format!("failed to write stdout: {error}")))?;
            stdout
                .flush()
                .map_err(|error| task_error(format!("failed to flush stdout: {error}")))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::StderrWrite { text } => {
            stderr
                .write_all(text.as_bytes())
                .map_err(|error| task_error(format!("failed to write stderr: {error}")))?;
            stderr
                .flush()
                .map_err(|error| task_error(format!("failed to flush stderr: {error}")))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsWriteText { path, text } => {
            fs::write(Path::new(path.as_ref()), text.as_ref())
                .map_err(|error| task_error(format!("failed to write {}: {error}", path)))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsWriteBytes { path, bytes } => {
            fs::write(Path::new(path.as_ref()), bytes.as_ref())
                .map_err(|error| task_error(format!("failed to write {}: {error}", path)))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsCreateDirAll { path } => {
            fs::create_dir_all(Path::new(path.as_ref()))
                .map_err(|error| task_error(format!("failed to create {}: {error}", path)))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsDeleteFile { path } => {
            fs::remove_file(Path::new(path.as_ref()))
                .map_err(|error| task_error(format!("failed to delete {}: {error}", path)))?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsReadText { path } => {
            let text = fs::read_to_string(Path::new(path.as_ref()))
                .map_err(|error| task_error(format!("failed to read {}: {error}", path)))?;
            Ok(RuntimeValue::Text(text.into()))
        }
        RuntimeTaskPlan::FsReadDir { path } => {
            let entries = fs::read_dir(Path::new(path.as_ref())).map_err(|error| {
                task_error(format!("failed to read directory {}: {error}", path))
            })?;
            let names: Result<Vec<RuntimeValue>, RuntimeTaskExecutionError> = entries
                .map(|entry| {
                    entry
                        .map_err(|error| {
                            task_error(format!("failed to read directory entry: {error}"))
                        })
                        .and_then(|entry| {
                            entry
                                .file_name()
                                .into_string()
                                .map(|name| RuntimeValue::Text(name.into()))
                                .map_err(|_| {
                                    task_error("directory entry name is not valid UTF-8".to_owned())
                                })
                        })
                })
                .collect();
            Ok(RuntimeValue::List(names?))
        }
        RuntimeTaskPlan::FsExists { path } => {
            Ok(RuntimeValue::Bool(Path::new(path.as_ref()).exists()))
        }
        RuntimeTaskPlan::FsReadBytes { path } => {
            let bytes = fs::read(Path::new(path.as_ref())).map_err(|error| {
                task_error(format!("failed to read bytes from {}: {error}", path))
            })?;
            Ok(RuntimeValue::Bytes(bytes.into()))
        }
        RuntimeTaskPlan::FsRename { from, to } => {
            fs::rename(Path::new(from.as_ref()), Path::new(to.as_ref())).map_err(|error| {
                task_error(format!("failed to rename {} to {}: {error}", from, to))
            })?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsCopy { from, to } => {
            fs::copy(Path::new(from.as_ref()), Path::new(to.as_ref())).map_err(|error| {
                task_error(format!("failed to copy {} to {}: {error}", from, to))
            })?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::FsDeleteDir { path } => {
            fs::remove_dir_all(Path::new(path.as_ref())).map_err(|error| {
                task_error(format!("failed to delete directory {}: {error}", path))
            })?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::JsonValidate { json } => {
            let valid = serde_json::from_str::<serde_json::Value>(&json).is_ok();
            Ok(RuntimeValue::Bool(valid))
        }
        RuntimeTaskPlan::JsonGet { json, key } => {
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| task_error(format!("json.get: invalid JSON: {error}")))?;
            Ok(parsed
                .get(key.as_ref())
                .map(|value| {
                    RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(value.to_string().into())))
                })
                .unwrap_or(RuntimeValue::OptionNone))
        }
        RuntimeTaskPlan::JsonAt { json, index } => {
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| task_error(format!("json.at: invalid JSON: {error}")))?;
            Ok(usize::try_from(index)
                .ok()
                .and_then(|index| parsed.get(index))
                .map(|value| {
                    RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(value.to_string().into())))
                })
                .unwrap_or(RuntimeValue::OptionNone))
        }
        RuntimeTaskPlan::JsonKeys { json } => {
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| task_error(format!("json.keys: invalid JSON: {error}")))?;
            let keys = parsed
                .as_object()
                .map(|object| {
                    object
                        .keys()
                        .map(|key| RuntimeValue::Text(key.as_str().into()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(RuntimeValue::List(keys))
        }
        RuntimeTaskPlan::JsonPretty { json } => {
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| task_error(format!("json.pretty: invalid JSON: {error}")))?;
            let pretty = serde_json::to_string_pretty(&parsed).map_err(|error| {
                task_error(format!("json.pretty: serialisation error: {error}"))
            })?;
            Ok(RuntimeValue::Text(pretty.into()))
        }
        RuntimeTaskPlan::JsonMinify { json } => {
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| task_error(format!("json.minify: invalid JSON: {error}")))?;
            let minified = serde_json::to_string(&parsed).map_err(|error| {
                task_error(format!("json.minify: serialisation error: {error}"))
            })?;
            Ok(RuntimeValue::Text(minified.into()))
        }
        // Time intrinsics
        RuntimeTaskPlan::TimeNowMs => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Ok(RuntimeValue::Int(ms))
        }
        RuntimeTaskPlan::TimeMonotonicMs => {
            static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let start = START.get_or_init(std::time::Instant::now);
            let ms = start.elapsed().as_millis() as i64;
            Ok(RuntimeValue::Int(ms))
        }
        RuntimeTaskPlan::TimeFormat {
            epoch_ms,
            pattern: _,
        } => {
            // Basic fallback: return epoch_ms as decimal text (chrono not available)
            Ok(RuntimeValue::Text(format!("{epoch_ms}").into()))
        }
        RuntimeTaskPlan::TimeParse { text, pattern: _ } => {
            // Basic fallback: try parsing as epoch ms integer string
            match text.trim().parse::<i64>() {
                Ok(ms) => Ok(RuntimeValue::Int(ms)),
                Err(_) => Err(task_error(format!("cannot parse timestamp: {}", text))),
            }
        }
        // Env intrinsics
        RuntimeTaskPlan::EnvGet { name } => Ok(match std::env::var(name.as_ref()) {
            Ok(val) => RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(val.into()))),
            Err(_) => RuntimeValue::OptionNone,
        }),
        RuntimeTaskPlan::EnvList { prefix } => {
            let pairs: Vec<RuntimeValue> = std::env::vars()
                .filter(|(k, _)| prefix.is_empty() || k.starts_with(prefix.as_ref()))
                .map(|(k, v)| {
                    RuntimeValue::Tuple(vec![
                        RuntimeValue::Text(k.into()),
                        RuntimeValue::Text(v.into()),
                    ])
                })
                .collect();
            Ok(RuntimeValue::List(pairs))
        }
        // Log intrinsics
        RuntimeTaskPlan::LogEmit { level, message } => {
            eprintln!("[{level}] {message}");
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::LogEmitContext {
            level,
            message,
            context,
        } => {
            let ctx: Vec<String> = context.iter().map(|(k, v)| format!("{k}={v}")).collect();
            eprintln!("[{level}] {message} {{{}}}", ctx.join(", "));
            Ok(RuntimeValue::Unit)
        }
        // Random float
        RuntimeTaskPlan::RandomFloat => {
            let bytes = read_os_random_bytes(8)?;
            let array: [u8; 8] = bytes
                .as_ref()
                .try_into()
                .map_err(|_| task_error("random float: unexpected byte buffer size"))?;
            let bits = u64::from_le_bytes(array);
            let f = (bits >> 11) as f64 / (1u64 << 53) as f64;
            RuntimeFloat::new(f)
                .map(RuntimeValue::Float)
                .ok_or_else(|| task_error("random float: result is not finite"))
        }
        // Regex intrinsics
        RuntimeTaskPlan::RegexIsMatch { pattern, text } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            Ok(RuntimeValue::Bool(re.is_match(text.as_ref())))
        }
        RuntimeTaskPlan::RegexFind { pattern, text } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            match re.find(text.as_ref()) {
                Some(m) => {
                    let char_idx = text[..m.start()].chars().count() as i64;
                    Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(
                        char_idx,
                    ))))
                }
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        RuntimeTaskPlan::RegexFindText { pattern, text } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            match re.find(text.as_ref()) {
                Some(m) => Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(
                    m.as_str().into(),
                )))),
                None => Ok(RuntimeValue::OptionNone),
            }
        }
        RuntimeTaskPlan::RegexFindAll { pattern, text } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            let matches: Vec<RuntimeValue> = re
                .find_iter(text.as_ref())
                .map(|m| RuntimeValue::Text(m.as_str().into()))
                .collect();
            Ok(RuntimeValue::List(matches))
        }
        RuntimeTaskPlan::RegexReplace {
            pattern,
            replacement,
            text,
        } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            Ok(RuntimeValue::Text(
                re.replacen(text.as_ref(), 1, replacement.as_ref())
                    .into_owned()
                    .into(),
            ))
        }
        RuntimeTaskPlan::RegexReplaceAll {
            pattern,
            replacement,
            text,
        } => {
            let re = Regex::new(pattern.as_ref()).map_err(|e| task_error(format!("regex: {e}")))?;
            Ok(RuntimeValue::Text(
                re.replace_all(text.as_ref(), replacement.as_ref())
                    .into_owned()
                    .into(),
            ))
        }
        RuntimeTaskPlan::HttpGet { url } => {
            let body = ureq::get(url.as_ref())
                .call()
                .map_err(|e| task_error(format!("http get: {e}")))?
                .into_string()
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Text(body.into()))
        }
        RuntimeTaskPlan::HttpGetBytes { url } => {
            let mut bytes = Vec::new();
            ureq::get(url.as_ref())
                .call()
                .map_err(|e| task_error(format!("http get: {e}")))?
                .into_reader()
                .read_to_end(&mut bytes)
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Bytes(bytes.into_boxed_slice()))
        }
        RuntimeTaskPlan::HttpGetStatus { url } => {
            let status = ureq::get(url.as_ref())
                .call()
                .map(|r| r.status() as i64)
                .unwrap_or_else(|e| match e {
                    ureq::Error::Status(code, _) => code as i64,
                    _ => 0,
                });
            Ok(RuntimeValue::Int(status))
        }
        RuntimeTaskPlan::HttpPost {
            url,
            content_type,
            body,
        } => {
            let response = ureq::post(url.as_ref())
                .set("Content-Type", content_type.as_ref())
                .send_string(body.as_ref())
                .map_err(|e| task_error(format!("http post: {e}")))?
                .into_string()
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Text(response.into()))
        }
        RuntimeTaskPlan::HttpPut {
            url,
            content_type,
            body,
        } => {
            let response = ureq::put(url.as_ref())
                .set("Content-Type", content_type.as_ref())
                .send_string(body.as_ref())
                .map_err(|e| task_error(format!("http put: {e}")))?
                .into_string()
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Text(response.into()))
        }
        RuntimeTaskPlan::HttpDelete { url } => {
            let response = ureq::delete(url.as_ref())
                .call()
                .map_err(|e| task_error(format!("http delete: {e}")))?
                .into_string()
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Text(response.into()))
        }
        RuntimeTaskPlan::HttpHead { url } => {
            let response = ureq::head(url.as_ref())
                .call()
                .map_err(|e| task_error(format!("http head: {e}")))?;
            let names = response.headers_names();
            let headers: Vec<RuntimeValue> = names
                .iter()
                .filter_map(|name| {
                    response.header(name).map(|val| {
                        RuntimeValue::Tuple(vec![
                            RuntimeValue::Text(name.clone().into()),
                            RuntimeValue::Text(val.into()),
                        ])
                    })
                })
                .collect();
            Ok(RuntimeValue::List(headers))
        }
        RuntimeTaskPlan::HttpPostJson { url, body } => {
            let response = ureq::post(url.as_ref())
                .set("Content-Type", "application/json")
                .send_string(body.as_ref())
                .map_err(|e| task_error(format!("http post json: {e}")))?
                .into_string()
                .map_err(|e| task_error(format!("http read: {e}")))?;
            Ok(RuntimeValue::Text(response.into()))
        }
        RuntimeTaskPlan::DbusCall {
            destination,
            path,
            interface,
            member,
            body,
            bus,
            address,
        } => {
            let connection = open_dbus_connection_text(
                bus.as_ref(),
                (!address.is_empty()).then_some(address.as_ref()),
            )
            .map_err(task_error)?;
            let body = runtime_dbus_values_to_variant(body.as_ref()).map_err(task_error)?;
            let reply = connection
                .call_sync(
                    Some(destination.as_ref()),
                    path.as_ref(),
                    interface.as_ref(),
                    member.as_ref(),
                    body.as_ref(),
                    None::<&glib::VariantTy>,
                    DBusCallFlags::NONE,
                    5_000,
                    None::<&gio::Cancellable>,
                )
                .map_err(runtime_dbus_call_error)?;
            Ok(RuntimeValue::List(
                runtime_dbus_body_from_variant(Some(&reply)).map_err(task_error)?,
            ))
        }
        RuntimeTaskPlan::SecretLookup {
            service,
            attributes,
        } => runtime_secret_lookup(service.as_ref(), attributes.as_ref()),
        RuntimeTaskPlan::SecretStore {
            service,
            label,
            attributes,
            value,
        } => {
            runtime_secret_store(
                service.as_ref(),
                label.as_ref(),
                attributes.as_ref(),
                value.as_ref(),
            )?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::SecretDelete {
            service,
            attributes,
        } => Ok(RuntimeValue::Bool(runtime_secret_delete(
            service.as_ref(),
            attributes.as_ref(),
        )?)),
        RuntimeTaskPlan::NotificationSend {
            app_name,
            notification,
            bus,
            address,
        } => runtime_notification_send(
            app_name.as_ref(),
            notification.as_ref(),
            bus.as_ref(),
            address.as_ref(),
        )
        .map(RuntimeValue::Int),
        RuntimeTaskPlan::NotificationClose {
            app_name,
            id,
            bus,
            address,
        } => {
            runtime_notification_close(app_name.as_ref(), id, bus.as_ref(), address.as_ref())?;
            Ok(RuntimeValue::Unit)
        }
        RuntimeTaskPlan::AuthPkce { config } => runtime_auth_pkce(config.as_ref()),
        RuntimeTaskPlan::AuthRefresh {
            config,
            refresh_token,
        } => runtime_auth_refresh(config.as_ref(), refresh_token.as_ref()),
        RuntimeTaskPlan::CustomCapabilityCommand(plan) => {
            let Some(executor) = context.custom_capability_command_executor() else {
                return Err(task_error(format!(
                    "custom capability command `{}.{}` has no registered executor",
                    plan.provider_key, plan.command
                )));
            };
            executor.execute(context, &plan, stdout, stderr)
        }
        // Invariant: Map/Apply/Chain/Join are deferred composition plans that require a
        // TaskFunctionApplier (a Cranelift evaluator). They must only be executed via
        // execute_runtime_task_plan_with_applier, never via this bare executor.
        RuntimeTaskPlan::Map { .. }
        | RuntimeTaskPlan::Apply { .. }
        | RuntimeTaskPlan::Chain { .. }
        | RuntimeTaskPlan::Join { .. } => {
            panic!(
                "BUG: deferred Task composition plan reached bare executor — \
                 these variants require an applier (execute_runtime_task_plan_with_applier)"
            )
        }
    }
}

pub fn execute_runtime_db_task_plan(
    plan: RuntimeDbTaskPlan,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_db_task_plan_with_effects(plan)?.value)
}

pub fn execute_runtime_task_plan_with_stdio(
    plan: RuntimeTaskPlan,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    execute_runtime_task_plan_with_context_with_stdio(plan, &SourceProviderContext::current())
}

pub fn execute_runtime_task_plan_with_context_with_stdio(
    plan: RuntimeTaskPlan,
    context: &SourceProviderContext,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    execute_runtime_task_plan_with_context(plan, context, &mut stdout, &mut stderr)
}

/// Execute a [`RuntimeTaskPlan`] with a [`TaskFunctionApplier`] callback that can apply user
/// closures. Required for deferred composition plans (`Map`, `Apply`, `Chain`, `Join`).
///
/// The `globals` map is passed to the applier so it can resolve item references inside closures.
pub(crate) fn execute_runtime_task_plan_with_applier(
    plan: RuntimeTaskPlan,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    applier: &mut dyn TaskFunctionApplier,
    globals: &BTreeMap<ItemId, RuntimeValue>,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    match plan {
        // Deferred composition variants — require the applier.
        RuntimeTaskPlan::Map { function, inner } => {
            let result = execute_runtime_task_plan_with_applier(
                *inner, context, stdout, stderr, applier, globals,
            )?;
            applier
                .apply_task_function(*function, vec![result], globals)
                .map_err(|e| RuntimeTaskExecutionError::new(format!("Task.map failed: {e}")))
        }
        RuntimeTaskPlan::Apply {
            function_task,
            value_task,
        } => {
            let function = execute_runtime_task_plan_with_applier(
                *function_task,
                context,
                stdout,
                stderr,
                applier,
                globals,
            )?;
            let value = execute_runtime_task_plan_with_applier(
                *value_task,
                context,
                stdout,
                stderr,
                applier,
                globals,
            )?;
            applier
                .apply_task_function(function, vec![value], globals)
                .map_err(|e| RuntimeTaskExecutionError::new(format!("Task.apply failed: {e}")))
        }
        RuntimeTaskPlan::Chain { function, inner } => {
            let result = execute_runtime_task_plan_with_applier(
                *inner, context, stdout, stderr, applier, globals,
            )?;
            let next_task = applier
                .apply_task_function(*function, vec![result], globals)
                .map_err(|e| RuntimeTaskExecutionError::new(format!("Task.chain failed: {e}")))?;
            match next_task {
                RuntimeValue::Task(next_plan) => execute_runtime_task_plan_with_applier(
                    next_plan, context, stdout, stderr, applier, globals,
                ),
                _ => Err(RuntimeTaskExecutionError::new(
                    "Task.chain: the continuation must return a Task value",
                )),
            }
        }
        RuntimeTaskPlan::Join { outer } => {
            let inner = execute_runtime_task_plan_with_applier(
                *outer, context, stdout, stderr, applier, globals,
            )?;
            match inner {
                RuntimeValue::Task(inner_plan) => execute_runtime_task_plan_with_applier(
                    inner_plan, context, stdout, stderr, applier, globals,
                ),
                _ => Err(RuntimeTaskExecutionError::new(
                    "Task.join: the outer task must produce a Task value",
                )),
            }
        }
        // All other variants delegate to the non-applier executor.
        other => execute_runtime_task_plan_with_context(other, context, stdout, stderr),
    }
}

/// Execute a [`RuntimeValue`] with an applier callback. If the value is a `Task` with deferred
/// composition plans, those are resolved using `applier` and `globals`.
pub(crate) fn execute_runtime_value_with_context_effects_and_applier(
    value: RuntimeValue,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    applier: &mut dyn TaskFunctionApplier,
    globals: &BTreeMap<ItemId, RuntimeValue>,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    match value {
        RuntimeValue::Task(plan) => {
            execute_runtime_task_plan_with_applier(plan, context, stdout, stderr, applier, globals)
                .map(RuntimeTaskExecutionOutcome::value)
        }
        RuntimeValue::DbTask(plan) => execute_runtime_db_task_plan_with_effects(plan),
        other => Ok(RuntimeTaskExecutionOutcome::value(other)),
    }
}

pub fn execute_runtime_value(
    value: RuntimeValue,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_context_effects(
        value,
        &SourceProviderContext::current(),
        stdout,
        stderr,
    )?
    .value)
}

pub fn execute_runtime_value_with_context(
    value: RuntimeValue,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_context_effects(value, context, stdout, stderr)?.value)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn execute_runtime_value_with_effects(
    value: RuntimeValue,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    execute_runtime_value_with_context_effects(
        value,
        &SourceProviderContext::current(),
        stdout,
        stderr,
    )
}

pub(crate) fn execute_runtime_value_with_context_effects(
    value: RuntimeValue,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    match value {
        RuntimeValue::Task(plan) => {
            execute_runtime_task_plan_with_context(plan, context, stdout, stderr)
                .map(RuntimeTaskExecutionOutcome::value)
        }
        RuntimeValue::DbTask(plan) => execute_runtime_db_task_plan_with_effects(plan),
        other => Ok(RuntimeTaskExecutionOutcome::value(other)),
    }
}

pub fn execute_runtime_value_with_stdio(
    value: RuntimeValue,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_context_with_stdio_effects(
        value,
        &SourceProviderContext::current(),
    )?
    .value)
}

pub fn execute_runtime_value_with_context_with_stdio(
    value: RuntimeValue,
    context: &SourceProviderContext,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_context_with_stdio_effects(value, context)?.value)
}

#[allow(dead_code)]
pub(crate) fn execute_runtime_value_with_stdio_effects(
    value: RuntimeValue,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    execute_runtime_value_with_context_with_stdio_effects(value, &SourceProviderContext::current())
}

pub(crate) fn execute_runtime_value_with_context_with_stdio_effects(
    value: RuntimeValue,
    context: &SourceProviderContext,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    execute_runtime_value_with_context_effects(value, context, &mut stdout, &mut stderr)
}

fn task_error(message: impl Into<String>) -> RuntimeTaskExecutionError {
    RuntimeTaskExecutionError::new(message)
}

fn runtime_dbus_call_error(error: glib::Error) -> RuntimeTaskExecutionError {
    use gio::DBusError;

    let message = error.message().to_string();
    let value = match error.kind::<DBusError>() {
        Some(DBusError::NameHasNoOwner) => {
            runtime_dbus_error("NameNotOwned", vec![RuntimeValue::Text(message.into())])
        }
        Some(DBusError::ServiceUnknown) => {
            runtime_dbus_error("ServiceUnknown", vec![RuntimeValue::Text(message.into())])
        }
        Some(DBusError::NoReply | DBusError::TimedOut | DBusError::Timeout) => {
            runtime_dbus_error("NoReply", Vec::new())
        }
        Some(DBusError::AccessDenied) => {
            runtime_dbus_error("AccessDenied", vec![RuntimeValue::Text(message.into())])
        }
        Some(DBusError::InvalidArgs) => {
            runtime_dbus_error("InvalidArgs", vec![RuntimeValue::Text(message.into())])
        }
        _ => runtime_dbus_error(
            "DbusProtocolError",
            vec![RuntimeValue::Text(message.into())],
        ),
    };
    RuntimeTaskExecutionError::new(value.to_string())
}

fn runtime_dbus_error(variant_name: &str, fields: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Sum(aivi_backend::RuntimeSumValue {
        item: aivi_hir::ItemId::from_raw(0),
        type_name: "DbusError".into(),
        variant_name: variant_name.into(),
        fields,
    })
}

const AIVI_SECRET_SERVICE_ATTRIBUTE: &str = "aivi.secret.service";
const FREEDESKTOP_NOTIFICATIONS_DESTINATION: &str = "org.freedesktop.Notifications";
const FREEDESKTOP_NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const FREEDESKTOP_NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";
const FREEDESKTOP_NOTIFICATIONS_DEFAULT_TIMEOUT_MS: i32 = -1;

fn runtime_secret_lookup(
    service: &str,
    attributes: &[(Box<str>, Box<str>)],
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let ss = runtime_secret_service()?;
    let mut items = runtime_secret_search_items(&ss, service, attributes)?;
    let Some(item) = items.pop() else {
        return Ok(RuntimeValue::OptionNone);
    };
    let secret = item.get_secret().map_err(runtime_secret_error)?;
    let secret = String::from_utf8(secret).map_err(|error| {
        RuntimeTaskExecutionError::new(runtime_secret_protocol_error(error.to_string()).to_string())
    })?;
    Ok(RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(
        secret.into_boxed_str(),
    ))))
}

fn runtime_secret_store(
    service: &str,
    label: &str,
    attributes: &[(Box<str>, Box<str>)],
    value: &str,
) -> Result<(), RuntimeTaskExecutionError> {
    let ss = runtime_secret_service()?;
    let collection = ss.get_default_collection().map_err(runtime_secret_error)?;
    collection
        .create_item(
            label,
            runtime_secret_attribute_map(service, attributes),
            value.as_bytes(),
            true,
            "text/plain",
        )
        .map_err(runtime_secret_error)?;
    Ok(())
}

fn runtime_secret_delete(
    service: &str,
    attributes: &[(Box<str>, Box<str>)],
) -> Result<bool, RuntimeTaskExecutionError> {
    let ss = runtime_secret_service()?;
    let items = runtime_secret_search_items(&ss, service, attributes)?;
    let had_items = !items.is_empty();
    for item in items {
        item.delete().map_err(runtime_secret_error)?;
    }
    Ok(had_items)
}

pub(crate) fn register_notification_id(app_name: &str, id: u32) {
    let mut registry = notification_registry()
        .lock()
        .expect("notification registry mutex should not be poisoned");
    registry.entry(app_name.into()).or_default().insert(id);
}

pub(crate) fn remove_notification_id(app_name: &str, id: u32) {
    let mut registry = notification_registry()
        .lock()
        .expect("notification registry mutex should not be poisoned");
    if let Some(ids) = registry.get_mut(app_name) {
        ids.remove(&id);
        if ids.is_empty() {
            registry.remove(app_name);
        }
    }
}

pub(crate) fn notification_id_known(app_name: &str, id: u32) -> bool {
    notification_registry()
        .lock()
        .expect("notification registry mutex should not be poisoned")
        .get(app_name)
        .is_some_and(|ids| ids.contains(&id))
}

fn notification_registry() -> &'static std::sync::Mutex<BTreeMap<Box<str>, BTreeSet<u32>>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<BTreeMap<Box<str>, BTreeSet<u32>>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

struct RuntimeNotificationAction {
    label: Box<str>,
    id: Box<str>,
}

struct RuntimeNotificationRequest {
    summary: Box<str>,
    body: Option<Box<str>>,
    icon: Option<Box<str>>,
    actions: Vec<RuntimeNotificationAction>,
}

fn runtime_notification_send(
    app_name: &str,
    notification: &RuntimeValue,
    bus: &str,
    address: &str,
) -> Result<i64, RuntimeTaskExecutionError> {
    let request = parse_notification_request(notification)?;
    let connection = open_dbus_connection_text(bus, (!address.is_empty()).then_some(address))
        .map_err(runtime_notification_error)?;
    let action_values = request
        .actions
        .iter()
        .flat_map(|action| {
            [
                action.id.as_ref().to_variant(),
                action.label.as_ref().to_variant(),
            ]
        })
        .collect::<Vec<_>>();
    let actions = Variant::array_from_iter_with_type(VariantTy::STRING, action_values.iter());
    let hints = Variant::parse(
        Some(VariantTy::new("a{sv}").expect("notification hints type should be valid")),
        "{}",
    )
    .map_err(runtime_notification_error)?;
    let reply = connection
        .call_sync(
            Some(FREEDESKTOP_NOTIFICATIONS_DESTINATION),
            FREEDESKTOP_NOTIFICATIONS_PATH,
            FREEDESKTOP_NOTIFICATIONS_INTERFACE,
            "Notify",
            Some(&Variant::tuple_from_iter([
                app_name.to_variant(),
                0_u32.to_variant(),
                request.icon.unwrap_or_default().to_variant(),
                request.summary.to_variant(),
                request.body.unwrap_or_default().to_variant(),
                actions,
                hints,
                FREEDESKTOP_NOTIFICATIONS_DEFAULT_TIMEOUT_MS.to_variant(),
            ])),
            None::<&glib::VariantTy>,
            DBusCallFlags::NONE,
            5_000,
            None::<&gio::Cancellable>,
        )
        .map_err(runtime_notification_error)?;
    let id = reply
        .child_value(0)
        .get::<u32>()
        .ok_or_else(|| runtime_notification_error("notification reply missing id"))?;
    register_notification_id(app_name, id);
    Ok(i64::from(id))
}

fn runtime_notification_close(
    app_name: &str,
    id: i64,
    bus: &str,
    address: &str,
) -> Result<(), RuntimeTaskExecutionError> {
    let id = u32::try_from(id)
        .map_err(|_| runtime_notification_error("notification id must fit in u32"))?;
    let connection = open_dbus_connection_text(bus, (!address.is_empty()).then_some(address))
        .map_err(runtime_notification_error)?;
    connection
        .call_sync(
            Some(FREEDESKTOP_NOTIFICATIONS_DESTINATION),
            FREEDESKTOP_NOTIFICATIONS_PATH,
            FREEDESKTOP_NOTIFICATIONS_INTERFACE,
            "CloseNotification",
            Some(&Variant::tuple_from_iter([id.to_variant()])),
            None::<&glib::VariantTy>,
            DBusCallFlags::NONE,
            5_000,
            None::<&gio::Cancellable>,
        )
        .map_err(runtime_notification_error)?;
    remove_notification_id(app_name, id);
    Ok(())
}

fn parse_notification_request(
    notification: &RuntimeValue,
) -> Result<RuntimeNotificationRequest, RuntimeTaskExecutionError> {
    let RuntimeValue::Record(fields) = notification else {
        return Err(runtime_notification_error(
            "notification payload must be a Notification record",
        ));
    };
    let summary = runtime_record_text_field(fields, "summary")?;
    let body = runtime_record_optional_text_field(fields, "body")?;
    let icon = runtime_record_optional_text_field(fields, "icon")?;
    let actions = match runtime_record_field(fields, "actions") {
        Some(RuntimeValue::List(actions)) => actions
            .iter()
            .map(parse_notification_action)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(runtime_notification_error(
                "notification payload field `actions` must be List NotificationAction",
            ));
        }
    };
    Ok(RuntimeNotificationRequest {
        summary,
        body,
        icon,
        actions,
    })
}

fn parse_notification_action(
    value: &RuntimeValue,
) -> Result<RuntimeNotificationAction, RuntimeTaskExecutionError> {
    let RuntimeValue::Record(fields) = value else {
        return Err(runtime_notification_error(
            "notification actions must be NotificationAction records",
        ));
    };
    Ok(RuntimeNotificationAction {
        label: runtime_record_text_field(fields, "label")?,
        id: runtime_record_text_field(fields, "id")?,
    })
}

fn runtime_secret_service() -> Result<SecretService<'static>, RuntimeTaskExecutionError> {
    SecretService::connect(EncryptionType::Dh).map_err(runtime_secret_error)
}

fn runtime_secret_search_items<'a>(
    ss: &'a SecretService<'a>,
    service: &str,
    attributes: &[(Box<str>, Box<str>)],
) -> Result<Vec<secret_service::blocking::Item<'a>>, RuntimeTaskExecutionError> {
    let search = ss
        .search_items(runtime_secret_attribute_map(service, attributes))
        .map_err(runtime_secret_error)?;
    let mut items = search.unlocked;
    for item in search.locked {
        item.unlock().map_err(runtime_secret_error)?;
        items.push(item);
    }
    Ok(items)
}

fn runtime_secret_attribute_map<'a>(
    service: &'a str,
    attributes: &'a [(Box<str>, Box<str>)],
) -> std::collections::HashMap<&'a str, &'a str> {
    let mut mapped = std::collections::HashMap::with_capacity(attributes.len() + 1);
    mapped.insert(AIVI_SECRET_SERVICE_ATTRIBUTE, service);
    for (key, value) in attributes {
        mapped.insert(key.as_ref(), value.as_ref());
    }
    mapped
}

fn runtime_secret_error(error: secret_service::Error) -> RuntimeTaskExecutionError {
    let value = match error {
        secret_service::Error::Unavailable => runtime_secret_sum(
            "SecretUnavailable",
            vec![RuntimeValue::Text("secret service unavailable".into())],
        ),
        secret_service::Error::Locked => runtime_secret_sum("SecretLocked", Vec::new()),
        secret_service::Error::Prompt => runtime_secret_sum("SecretCancelled", Vec::new()),
        other => runtime_secret_protocol_error(other.to_string()),
    };
    RuntimeTaskExecutionError::new(value.to_string())
}

fn runtime_secret_protocol_error(message: String) -> RuntimeValue {
    runtime_secret_sum(
        "SecretProtocolError",
        vec![RuntimeValue::Text(message.into_boxed_str())],
    )
}

fn runtime_secret_sum(variant_name: &str, fields: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Sum(aivi_backend::RuntimeSumValue {
        item: aivi_hir::ItemId::from_raw(0),
        type_name: "SecretError".into(),
        variant_name: variant_name.into(),
        fields,
    })
}

const PKCE_CALLBACK_HOST: &str = "127.0.0.1";
const PKCE_CALLBACK_PATH: &str = "/callback";
const PKCE_TIMEOUT: Duration = Duration::from_secs(180);

struct RuntimePkceConfig {
    client_id: Box<str>,
    auth_endpoint: Box<str>,
    token_endpoint: Box<str>,
    scopes: Vec<Box<str>>,
    redirect_port: u16,
}

fn runtime_auth_pkce(config: &RuntimeValue) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let config = parse_pkce_config(config)?;
    let redirect_uri = pkce_redirect_uri(config.redirect_port);
    let state = pkce_random_token(24);
    let verifier = pkce_random_token(64);
    let challenge = pkce_code_challenge(&verifier);
    let listener = TcpListener::bind((PKCE_CALLBACK_HOST, config.redirect_port))
        .map_err(runtime_pkce_network_error)?;
    listener
        .set_nonblocking(true)
        .map_err(runtime_pkce_network_error)?;

    let mut auth_url = Url::parse(config.auth_endpoint.as_ref())
        .map_err(|error| runtime_pkce_invalid_response(error.to_string()))?;
    {
        let mut query = auth_url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", config.client_id.as_ref());
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("scope", &config.scopes.join(" "));
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", &state);
    }

    runtime_launch_browser(auth_url.as_ref())?;
    let callback = runtime_wait_for_pkce_callback(&listener)?;
    if let Some(error) = callback.error {
        return match error.as_ref() {
            "access_denied" => Err(runtime_pkce_error("UserCancelled", Vec::new())),
            _ => Err(runtime_pkce_invalid_response(error.into_string())),
        };
    }
    if callback.state != state {
        return Err(runtime_pkce_invalid_response(
            "PKCE callback state mismatch".to_owned(),
        ));
    }
    let code = callback
        .code
        .ok_or_else(|| runtime_pkce_invalid_response("PKCE callback missing code".to_owned()))?;

    runtime_exchange_pkce_token(
        &config,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_ref()),
            ("code", code.as_ref()),
            ("redirect_uri", redirect_uri.as_ref()),
            ("code_verifier", verifier.as_ref()),
        ],
        None,
    )
}

fn runtime_auth_refresh(
    config: &RuntimeValue,
    refresh_token: &str,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let config = parse_pkce_config(config)?;
    runtime_exchange_pkce_token(
        &config,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", config.client_id.as_ref()),
            ("refresh_token", refresh_token),
        ],
        Some(refresh_token),
    )
}

fn runtime_exchange_pkce_token(
    config: &RuntimePkceConfig,
    params: &[(&str, &str)],
    fallback_refresh_token: Option<&str>,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().copied())
        .finish();
    let response = ureq::post(config.token_endpoint.as_ref())
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .map_err(|error| runtime_pkce_network_error(error.to_string()))?;
    let payload = response.into_string().map_err(runtime_pkce_network_error)?;
    let json: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|error| runtime_pkce_invalid_response(error.to_string()))?;
    runtime_pkce_token_from_json(&json, fallback_refresh_token)
}

struct PkceCallback {
    code: Option<Box<str>>,
    state: Box<str>,
    error: Option<Box<str>>,
}

fn runtime_wait_for_pkce_callback(
    listener: &TcpListener,
) -> Result<PkceCallback, RuntimeTaskExecutionError> {
    let deadline = Instant::now() + PKCE_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut request = String::new();
                {
                    let mut reader = BufReader::new(&mut stream);
                    loop {
                        let mut line = String::new();
                        let bytes = reader
                            .read_line(&mut line)
                            .map_err(runtime_pkce_network_error)?;
                        if bytes == 0 {
                            break;
                        }
                        request.push_str(&line);
                        if line == "\r\n" {
                            break;
                        }
                    }
                }
                let callback = parse_pkce_callback_request(&request)?;
                let response_body = if callback.error.is_some() {
                    "Authentication failed. You can close this window."
                } else {
                    "Authentication complete. You can close this window."
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .map_err(runtime_pkce_network_error)?;
                return Ok(callback);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(runtime_pkce_error("PkceTimeout", Vec::new()));
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(runtime_pkce_network_error(error)),
        }
    }
}

fn parse_pkce_callback_request(request: &str) -> Result<PkceCallback, RuntimeTaskExecutionError> {
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| runtime_pkce_invalid_response("empty PKCE callback request".to_owned()))?;
    let path = first_line.split_whitespace().nth(1).ok_or_else(|| {
        runtime_pkce_invalid_response("malformed PKCE callback request".to_owned())
    })?;
    let url = Url::parse(&format!("http://{PKCE_CALLBACK_HOST}{path}"))
        .map_err(|error| runtime_pkce_invalid_response(error.to_string()))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned().into_boxed_str()),
            "state" => state = Some(value.into_owned().into_boxed_str()),
            "error" => error = Some(value.into_owned().into_boxed_str()),
            _ => {}
        }
    }
    Ok(PkceCallback {
        code,
        state: state.ok_or_else(|| {
            runtime_pkce_invalid_response("PKCE callback missing state".to_owned())
        })?,
        error,
    })
}

fn runtime_launch_browser(url: &str) -> Result<(), RuntimeTaskExecutionError> {
    #[cfg(test)]
    {
        let opener = TEST_BROWSER_OPENER
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .expect("test browser opener mutex should not be poisoned")
            .as_ref()
            .copied();
        if let Some(opener) = opener {
            opener(url).map_err(runtime_pkce_network_error)?;
            return Ok(());
        }
    }
    let status = Command::new("xdg-open")
        .arg(url)
        .status()
        .map_err(runtime_pkce_network_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(runtime_pkce_network_error(format!(
            "xdg-open exited with status {status}"
        )))
    }
}

fn parse_pkce_config(
    config: &RuntimeValue,
) -> Result<RuntimePkceConfig, RuntimeTaskExecutionError> {
    let RuntimeValue::Record(fields) = config else {
        return Err(runtime_pkce_invalid_response(
            "PKCE config must be a record".to_owned(),
        ));
    };
    let client_id = runtime_record_text_field(fields, "clientId")?;
    let auth_endpoint = runtime_record_text_field(fields, "authEndpoint")?;
    let token_endpoint = runtime_record_text_field(fields, "tokenEndpoint")?;
    let scopes_value = runtime_record_field(fields, "scopes")
        .ok_or_else(|| runtime_pkce_invalid_response("PKCE config missing scopes".to_owned()))?;
    let RuntimeValue::List(scopes) = scopes_value else {
        return Err(runtime_pkce_invalid_response(
            "PKCE config scopes must be List Text".to_owned(),
        ));
    };
    let scopes = scopes
        .iter()
        .map(|scope| match scope {
            RuntimeValue::Text(text) => Ok(text.clone()),
            _ => Err(runtime_pkce_invalid_response(
                "PKCE config scopes must be List Text".to_owned(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let redirect_port = runtime_record_int_field(fields, "redirectPort")?;
    let redirect_port = u16::try_from(redirect_port).map_err(|_| {
        runtime_pkce_invalid_response("PKCE redirectPort must fit in u16".to_owned())
    })?;
    Ok(RuntimePkceConfig {
        client_id,
        auth_endpoint,
        token_endpoint,
        scopes,
        redirect_port,
    })
}

fn runtime_pkce_token_from_json(
    json: &serde_json::Value,
    fallback_refresh_token: Option<&str>,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let Some(access_token) = json.get("access_token").and_then(serde_json::Value::as_str) else {
        return Err(runtime_pkce_invalid_response(
            "token response missing access_token".to_owned(),
        ));
    };
    let refresh_token = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .map(|token| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(token.into()))))
        .or_else(|| {
            fallback_refresh_token
                .map(|token| RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(token.into()))))
        })
        .unwrap_or(RuntimeValue::OptionNone);
    let expires_at = json
        .get("expires_in")
        .and_then(serde_json::Value::as_i64)
        .map(|seconds| {
            RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(
                current_unix_ms().saturating_add(seconds.saturating_mul(1000)),
            )))
        })
        .unwrap_or(RuntimeValue::OptionNone);
    Ok(RuntimeValue::Record(vec![
        aivi_backend::RuntimeRecordField {
            label: "accessToken".into(),
            value: RuntimeValue::Text(access_token.into()),
        },
        aivi_backend::RuntimeRecordField {
            label: "refreshToken".into(),
            value: refresh_token,
        },
        aivi_backend::RuntimeRecordField {
            label: "expiresAt".into(),
            value: expires_at,
        },
    ]))
}

fn runtime_record_field<'a>(
    fields: &'a [aivi_backend::RuntimeRecordField],
    label: &str,
) -> Option<&'a RuntimeValue> {
    fields
        .iter()
        .find(|field| field.label.as_ref() == label)
        .map(|field| &field.value)
}

fn runtime_record_text_field(
    fields: &[aivi_backend::RuntimeRecordField],
    label: &str,
) -> Result<Box<str>, RuntimeTaskExecutionError> {
    match runtime_record_field(fields, label) {
        Some(RuntimeValue::Text(text)) => Ok(text.clone()),
        _ => Err(runtime_pkce_invalid_response(format!(
            "PKCE config field `{label}` must be Text"
        ))),
    }
}

fn runtime_record_optional_text_field(
    fields: &[aivi_backend::RuntimeRecordField],
    label: &str,
) -> Result<Option<Box<str>>, RuntimeTaskExecutionError> {
    match runtime_record_field(fields, label) {
        Some(RuntimeValue::OptionSome(value)) => match value.as_ref() {
            RuntimeValue::Text(text) => Ok(Some(text.clone())),
            _ => Err(runtime_notification_error(format!(
                "notification payload field `{label}` must be Option Text"
            ))),
        },
        Some(RuntimeValue::OptionNone) => Ok(None),
        _ => Err(runtime_notification_error(format!(
            "notification payload field `{label}` must be Option Text"
        ))),
    }
}

fn runtime_record_int_field(
    fields: &[aivi_backend::RuntimeRecordField],
    label: &str,
) -> Result<i64, RuntimeTaskExecutionError> {
    match runtime_record_field(fields, label) {
        Some(RuntimeValue::Int(value)) => Ok(*value),
        _ => Err(runtime_pkce_invalid_response(format!(
            "PKCE config field `{label}` must be Int"
        ))),
    }
}

fn pkce_redirect_uri(port: u16) -> Box<str> {
    format!("http://{PKCE_CALLBACK_HOST}:{port}{PKCE_CALLBACK_PATH}").into_boxed_str()
}

fn pkce_random_token(length: usize) -> Box<str> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let token = (0..length)
        .map(|_| {
            let index = fastrand::usize(..ALPHABET.len());
            ALPHABET[index] as char
        })
        .collect::<String>();
    token.into_boxed_str()
}

fn pkce_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn runtime_pkce_network_error(error: impl fmt::Display) -> RuntimeTaskExecutionError {
    runtime_pkce_error(
        "NetworkError",
        vec![RuntimeValue::Text(error.to_string().into_boxed_str())],
    )
}

fn runtime_notification_error(error: impl fmt::Display) -> RuntimeTaskExecutionError {
    RuntimeTaskExecutionError::new(format!("NotificationFailed {error}"))
}

fn runtime_pkce_invalid_response(message: String) -> RuntimeTaskExecutionError {
    runtime_pkce_error(
        "InvalidResponse",
        vec![RuntimeValue::Text(message.into_boxed_str())],
    )
}

fn runtime_pkce_error(variant_name: &str, fields: Vec<RuntimeValue>) -> RuntimeTaskExecutionError {
    RuntimeTaskExecutionError::new(
        RuntimeValue::Sum(aivi_backend::RuntimeSumValue {
            item: aivi_hir::ItemId::from_raw(0),
            type_name: "PkceError".into(),
            variant_name: variant_name.into(),
            fields,
        })
        .to_string(),
    )
}

#[cfg(test)]
static TEST_BROWSER_OPENER: std::sync::OnceLock<
    std::sync::Mutex<Option<fn(&str) -> Result<(), String>>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn set_test_browser_opener(opener: Option<fn(&str) -> Result<(), String>>) {
    *TEST_BROWSER_OPENER
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test browser opener mutex should not be poisoned") = opener;
}

fn execute_runtime_db_task_plan_with_effects(
    plan: RuntimeDbTaskPlan,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    match plan {
        RuntimeDbTaskPlan::Query(plan) => {
            execute_runtime_db_query_plan(plan).map(RuntimeTaskExecutionOutcome::value)
        }
        RuntimeDbTaskPlan::Commit(plan) => execute_runtime_db_commit_plan(plan),
    }
}

fn execute_runtime_db_query_plan(
    plan: RuntimeDbQueryPlan,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let script = sqlite_query_script(&plan.statement)?;
    let output = run_sqlite3_script(plan.connection.database.as_ref(), &script)?;
    if !output.status.success() {
        return Ok(db_task_error_value(sqlite_output_error(&output)));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        task_error(format!("sqlite3 query output was not valid UTF-8: {error}"))
    })?;
    let payload = if stdout.trim().is_empty() {
        serde_json::Value::Array(Vec::new())
    } else {
        serde_json::from_str(stdout.trim()).map_err(|error| {
            task_error(format!("sqlite3 query output was not valid JSON: {error}"))
        })?
    };
    Ok(RuntimeValue::ResultOk(Box::new(decode_sqlite_rows(
        payload,
    )?)))
}

fn execute_runtime_db_commit_plan(
    plan: RuntimeDbCommitPlan,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    if plan.statements.is_empty() {
        return Ok(RuntimeTaskExecutionOutcome {
            value: RuntimeValue::ResultOk(Box::new(RuntimeValue::Unit)),
            commit_invalidation: Some(RuntimeDbCommitInvalidation {
                connection: plan.connection,
                changed_tables: plan.changed_tables,
            }),
        });
    }
    let script = sqlite_commit_script(&plan)?;
    let output = run_sqlite3_script(plan.connection.database.as_ref(), &script)?;
    if output.status.success() {
        Ok(RuntimeTaskExecutionOutcome {
            value: RuntimeValue::ResultOk(Box::new(RuntimeValue::Unit)),
            commit_invalidation: Some(RuntimeDbCommitInvalidation {
                connection: plan.connection,
                changed_tables: plan.changed_tables,
            }),
        })
    } else {
        Ok(RuntimeTaskExecutionOutcome::value(db_task_error_value(
            sqlite_output_error(&output),
        )))
    }
}

fn sqlite_query_script(
    statement: &RuntimeDbStatement,
) -> Result<String, RuntimeTaskExecutionError> {
    let mut script = String::from(".bail on\n.parameter init\n.parameter clear\n.mode json\n");
    append_sqlite_parameter_bindings(&mut script, &statement.arguments)?;
    append_sqlite_statement(&mut script, &statement.sql);
    Ok(script)
}

fn sqlite_commit_script(plan: &RuntimeDbCommitPlan) -> Result<String, RuntimeTaskExecutionError> {
    let mut script = String::from(".bail on\n.parameter init\nBEGIN IMMEDIATE;\n");
    for statement in &plan.statements {
        script.push_str(".parameter clear\n");
        append_sqlite_parameter_bindings(&mut script, &statement.arguments)?;
        append_sqlite_statement(&mut script, &statement.sql);
    }
    script.push_str("COMMIT;\n");
    Ok(script)
}

fn append_sqlite_parameter_bindings(
    script: &mut String,
    arguments: &[RuntimeValue],
) -> Result<(), RuntimeTaskExecutionError> {
    for (index, argument) in arguments.iter().enumerate() {
        let literal = sqlite_parameter_literal(argument).map_err(task_error)?;
        script.push_str(".parameter set ?");
        script.push_str(&(index + 1).to_string());
        script.push(' ');
        script.push_str(&literal);
        script.push('\n');
    }
    Ok(())
}

fn append_sqlite_statement(script: &mut String, sql: &str) {
    script.push_str(sql);
    if !sql.trim_end().ends_with(';') {
        script.push(';');
    }
    script.push('\n');
}

fn sqlite_parameter_literal(value: &RuntimeValue) -> Result<String, String> {
    match strip_runtime_signal(value) {
        RuntimeValue::Unit | RuntimeValue::OptionNone => Ok("NULL".to_owned()),
        RuntimeValue::Bool(value) => Ok(if *value { "1" } else { "0" }.to_owned()),
        RuntimeValue::Int(value) => Ok(value.to_string()),
        RuntimeValue::Float(value) => Ok(value.to_f64().to_string()),
        RuntimeValue::Decimal(value) => Ok(sqlite_text_literal(&value.to_string())),
        RuntimeValue::BigInt(value) => Ok(sqlite_text_literal(&value.to_string())),
        RuntimeValue::Text(value) => Ok(sqlite_text_literal(value)),
        RuntimeValue::Bytes(bytes) => Ok(sqlite_blob_literal(bytes)),
        other => Err(format!(
            "sqlite parameter binding currently supports Unit/None, Bool, Int, Float, Decimal, BigInt, Text, and Bytes, found {other}"
        )),
    }
}

fn sqlite_text_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sqlite_blob_literal(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2 + 3);
    encoded.push_str("X'");
    for byte in bytes {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }
    encoded.push('\'');
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'A' + (value - 10)),
        _ => unreachable!("hex nybbles must stay within 0..=15"),
    }
}

fn run_sqlite3_script(database: &str, script: &str) -> Result<Output, RuntimeTaskExecutionError> {
    let mut child = Command::new("sqlite3")
        .arg(database)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| task_error(format!("failed to start sqlite3: {error}")))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| task_error("failed to open sqlite3 stdin".to_owned()))?;
        stdin
            .write_all(script.as_bytes())
            .map_err(|error| task_error(format!("failed to write sqlite3 script: {error}")))?;
    }
    child
        .wait_with_output()
        .map_err(|error| task_error(format!("failed to wait for sqlite3: {error}")))
}

fn sqlite_output_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("sqlite3 exited with status {}", output.status)
    } else {
        stderr
    }
}

fn decode_sqlite_rows(
    payload: serde_json::Value,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    let serde_json::Value::Array(rows) = payload else {
        return Err(task_error(
            "sqlite query output must be a JSON array of row objects".to_owned(),
        ));
    };
    let mut decoded_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let serde_json::Value::Object(fields) = row else {
            return Err(task_error(
                "sqlite query output must contain only row objects".to_owned(),
            ));
        };
        let entries = fields
            .into_iter()
            .map(|(key, value)| RuntimeMapEntry {
                key: RuntimeValue::Text(key.into_boxed_str()),
                value: RuntimeValue::Text(sqlite_json_value_to_text(value).into_boxed_str()),
            })
            .collect::<Vec<_>>();
        decoded_rows.push(RuntimeValue::Map(RuntimeMap::from_entries(entries)));
    }
    Ok(RuntimeValue::List(decoded_rows))
}

fn sqlite_json_value_to_text(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value,
        other => other.to_string(),
    }
}

fn db_task_error_value(message: String) -> RuntimeValue {
    RuntimeValue::ResultErr(Box::new(RuntimeValue::Text(message.into_boxed_str())))
}

fn strip_runtime_signal(value: &RuntimeValue) -> &RuntimeValue {
    let mut current = value;
    while let RuntimeValue::Signal(inner) = current {
        current = inner.as_ref();
    }
    current
}

fn sample_random_i64_inclusive(low: i64, high: i64) -> Result<i64, RuntimeTaskExecutionError> {
    if low > high {
        return Err(task_error(format!(
            "randomInt requires `low <= high`, found low={low} and high={high}"
        )));
    }
    if low == i64::MIN && high == i64::MAX {
        return Ok(i64::from_le_bytes(random_u64()?.to_le_bytes()));
    }
    let range = u128::try_from(i128::from(high) - i128::from(low) + 1)
        .expect("inclusive i64 range should fit into u128");
    let domain = u128::from(u64::MAX) + 1;
    let limit = (domain / range) * range;
    loop {
        let candidate = u128::from(random_u64()?);
        if candidate < limit {
            let value = i128::from(low)
                + i128::try_from(candidate % range)
                    .expect("random range remainder should fit into i128");
            return Ok(i64::try_from(value).expect("random value should remain within i64 bounds"));
        }
    }
}

fn random_u64() -> Result<u64, RuntimeTaskExecutionError> {
    let bytes = read_os_random_bytes(std::mem::size_of::<u64>())?;
    let array: [u8; std::mem::size_of::<u64>()] = bytes
        .as_ref()
        .try_into()
        .expect("fixed-length random byte buffer should match u64 width");
    Ok(u64::from_le_bytes(array))
}

fn read_os_random_bytes(count: usize) -> Result<Box<[u8]>, RuntimeTaskExecutionError> {
    let mut file = fs::File::open("/dev/urandom")
        .map_err(|error| task_error(format!("failed to open /dev/urandom: {error}")))?;
    let mut bytes = vec![0u8; count];
    if count > 0 {
        file.read_exact(&mut bytes).map_err(|error| {
            task_error(format!("failed to read {count} random byte(s): {error}"))
        })?;
    }
    Ok(bytes.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{Arc, Mutex, OnceLock},
        thread,
    };

    use aivi_backend::{
        RuntimeCustomCapabilityCommandPlan, RuntimeDbCommitPlan, RuntimeDbConnection,
        RuntimeDbQueryPlan, RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeMap, RuntimeMapEntry,
        RuntimeNamedValue, RuntimeTaskPlan, RuntimeValue,
    };

    use super::{
        CustomCapabilityCommandExecutor, RuntimeDbCommitInvalidation, execute_runtime_task_plan,
        execute_runtime_task_plan_with_context, execute_runtime_value,
        execute_runtime_value_with_effects,
    };
    use crate::SourceProviderContext;

    static AUTH_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[derive(Default)]
    struct EchoCustomCapabilityCommandExecutor;

    impl CustomCapabilityCommandExecutor for EchoCustomCapabilityCommandExecutor {
        fn execute(
            &self,
            _context: &SourceProviderContext,
            plan: &RuntimeCustomCapabilityCommandPlan,
            _stdout: &mut dyn std::io::Write,
            _stderr: &mut dyn std::io::Write,
        ) -> Result<RuntimeValue, super::RuntimeTaskExecutionError> {
            assert_eq!(plan.provider_key.as_ref(), "custom.feed");
            assert_eq!(plan.command.as_ref(), "delete");
            assert_eq!(
                plan.provider_arguments.as_ref(),
                [RuntimeNamedValue {
                    name: "root".into(),
                    value: RuntimeValue::Text("/tmp/demo".into()),
                }]
            );
            assert_eq!(
                plan.options.as_ref(),
                [RuntimeNamedValue {
                    name: "mode".into(),
                    value: RuntimeValue::Text("sync".into()),
                }]
            );
            assert_eq!(
                plan.arguments.as_ref(),
                [RuntimeNamedValue {
                    name: "arg1".into(),
                    value: RuntimeValue::Text("config".into()),
                }]
            );
            Ok(RuntimeValue::Text("deleted".into()))
        }
    }

    fn test_path(prefix: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-scratch");
        fs::create_dir_all(&base).expect("runtime task scratch directory should exist");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        base.join(format!(
            "aivi-runtime-task-{prefix}-{}-{unique}.sqlite",
            std::process::id()
        ))
    }

    fn auth_test_lock() -> std::sync::MutexGuard<'static, ()> {
        AUTH_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("auth test mutex should not be poisoned")
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .expect("request line should read");
            if bytes == 0 {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().expect("content-length should parse");
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            reader
                .read_exact(&mut body)
                .expect("request body should read");
            request.push_str(&String::from_utf8(body).expect("request body should be utf-8"));
        }
        request
    }

    fn pkce_config(auth_endpoint: &str, token_endpoint: &str, redirect_port: u16) -> RuntimeValue {
        RuntimeValue::Record(vec![
            aivi_backend::RuntimeRecordField {
                label: "clientId".into(),
                value: RuntimeValue::Text("desktop-client".into()),
            },
            aivi_backend::RuntimeRecordField {
                label: "authEndpoint".into(),
                value: RuntimeValue::Text(auth_endpoint.into()),
            },
            aivi_backend::RuntimeRecordField {
                label: "tokenEndpoint".into(),
                value: RuntimeValue::Text(token_endpoint.into()),
            },
            aivi_backend::RuntimeRecordField {
                label: "scopes".into(),
                value: RuntimeValue::List(vec![
                    RuntimeValue::Text("mail.read".into()),
                    RuntimeValue::Text("mail.send".into()),
                ]),
            },
            aivi_backend::RuntimeRecordField {
                label: "redirectPort".into(),
                value: RuntimeValue::Int(i64::from(redirect_port)),
            },
        ])
    }

    #[test]
    fn execute_runtime_task_value_passes_through_non_task_values() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_value(RuntimeValue::Int(42), &mut stdout, &mut stderr)
            .expect("plain runtime values should pass through unchanged");

        assert_eq!(result, RuntimeValue::Int(42));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_writes_to_supplied_stdout() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_task_plan(
            RuntimeTaskPlan::StdoutWrite {
                text: "hello".into(),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("stdout task should execute");

        assert_eq!(result, RuntimeValue::Unit);
        assert_eq!(stdout, b"hello");
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_returns_pure_payload() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_task_plan(
            RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::Bool(true)),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("pure task should execute");

        assert_eq!(result, RuntimeValue::Bool(true));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_reports_invalid_random_ranges() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = execute_runtime_task_plan(
            RuntimeTaskPlan::RandomInt { low: 9, high: 3 },
            &mut stdout,
            &mut stderr,
        )
        .expect_err("invalid random ranges should fail");

        assert_eq!(
            error.to_string(),
            "randomInt requires `low <= high`, found low=9 and high=3"
        );
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_reports_missing_custom_command_executor() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = execute_runtime_task_plan(
            RuntimeTaskPlan::CustomCapabilityCommand(RuntimeCustomCapabilityCommandPlan {
                provider_key: "custom.feed".into(),
                command: "delete".into(),
                provider_arguments: Box::new([]),
                options: Box::new([]),
                arguments: Box::new([]),
            }),
            &mut stdout,
            &mut stderr,
        )
        .expect_err("custom capability commands should fail clearly without a registered executor");

        assert_eq!(
            error.to_string(),
            "custom capability command `custom.feed.delete` has no registered executor"
        );
    }

    #[test]
    fn execute_runtime_task_plan_runs_custom_capability_commands_through_registered_executor() {
        let context = SourceProviderContext::current()
            .with_custom_capability_command_executor(Arc::new(EchoCustomCapabilityCommandExecutor));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let value = execute_runtime_task_plan_with_context(
            RuntimeTaskPlan::CustomCapabilityCommand(RuntimeCustomCapabilityCommandPlan {
                provider_key: "custom.feed".into(),
                command: "delete".into(),
                provider_arguments: vec![RuntimeNamedValue {
                    name: "root".into(),
                    value: RuntimeValue::Text("/tmp/demo".into()),
                }]
                .into_boxed_slice(),
                options: vec![RuntimeNamedValue {
                    name: "mode".into(),
                    value: RuntimeValue::Text("sync".into()),
                }]
                .into_boxed_slice(),
                arguments: vec![RuntimeNamedValue {
                    name: "arg1".into(),
                    value: RuntimeValue::Text("config".into()),
                }]
                .into_boxed_slice(),
            }),
            &context,
            &mut stdout,
            &mut stderr,
        )
        .expect("custom capability commands should run through the registered executor");

        assert_eq!(value, RuntimeValue::Text("deleted".into()));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_value_runs_db_commit_and_query_tasks() {
        let database = test_path("db-query");
        let connection = RuntimeDbConnection {
            database: database.to_string_lossy().into_owned().into_boxed_str(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let commit = RuntimeValue::DbTask(RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: connection.clone(),
            statements: vec![
                RuntimeDbStatement {
                    sql: "create table users(id integer primary key, name text not null)".into(),
                    arguments: Vec::new(),
                },
                RuntimeDbStatement {
                    sql: "insert into users(id, name) values (?, ?)".into(),
                    arguments: vec![RuntimeValue::Int(1), RuntimeValue::Text("Ada".into())],
                },
                RuntimeDbStatement {
                    sql: "insert into users(id, name) values (?, ?)".into(),
                    arguments: vec![RuntimeValue::Int(2), RuntimeValue::Text("Linus".into())],
                },
            ],
            changed_tables: BTreeSet::from(["users".into()]),
        }));

        let commit_result = execute_runtime_value(commit, &mut stdout, &mut stderr)
            .expect("db commit task should execute");
        assert_eq!(
            commit_result,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Unit))
        );

        let query_result = execute_runtime_value(
            RuntimeValue::DbTask(RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
                connection,
                statement: RuntimeDbStatement {
                    sql: "select id, name from users order by id".into(),
                    arguments: Vec::new(),
                },
            })),
            &mut stdout,
            &mut stderr,
        )
        .expect("db query task should execute");

        assert_eq!(
            query_result,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::List(vec![
                RuntimeValue::Map(RuntimeMap::from_entries(vec![
                    RuntimeMapEntry {
                        key: RuntimeValue::Text("id".into()),
                        value: RuntimeValue::Text("1".into()),
                    },
                    RuntimeMapEntry {
                        key: RuntimeValue::Text("name".into()),
                        value: RuntimeValue::Text("Ada".into()),
                    },
                ])),
                RuntimeValue::Map(RuntimeMap::from_entries(vec![
                    RuntimeMapEntry {
                        key: RuntimeValue::Text("id".into()),
                        value: RuntimeValue::Text("2".into()),
                    },
                    RuntimeMapEntry {
                        key: RuntimeValue::Text("name".into()),
                        value: RuntimeValue::Text("Linus".into()),
                    },
                ])),
            ]))),
        );
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        let _ = fs::remove_file(&database);
    }

    #[test]
    fn execute_runtime_value_surfaces_db_query_failures_as_result_err_text() {
        let database = test_path("db-query-failure");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_value(
            RuntimeValue::DbTask(RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
                connection: RuntimeDbConnection {
                    database: database.to_string_lossy().into_owned().into_boxed_str(),
                },
                statement: RuntimeDbStatement {
                    sql: "select id from missing_table".into(),
                    arguments: Vec::new(),
                },
            })),
            &mut stdout,
            &mut stderr,
        )
        .expect("db query task should return a result value");

        let RuntimeValue::ResultErr(error) = result else {
            panic!("expected db query failure result, found {result:?}");
        };
        let RuntimeValue::Text(message) = error.as_ref() else {
            panic!("expected db query failure message text, found {error:?}");
        };
        assert!(
            message.contains("no such table"),
            "expected missing-table error text, found {message}"
        );
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        let _ = fs::remove_file(&database);
    }

    #[test]
    fn execute_runtime_value_reports_db_commit_invalidation_on_success() {
        let database = test_path("db-commit-invalidation-success");
        let connection = RuntimeDbConnection {
            database: database.to_string_lossy().into_owned().into_boxed_str(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = execute_runtime_value_with_effects(
            RuntimeValue::DbTask(RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
                connection: connection.clone(),
                statements: vec![RuntimeDbStatement {
                    sql: "create table users(id integer primary key, name text not null)".into(),
                    arguments: Vec::new(),
                }],
                changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
            })),
            &mut stdout,
            &mut stderr,
        )
        .expect("db commit task should execute");

        assert_eq!(
            outcome.value,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Unit))
        );
        assert_eq!(
            outcome.commit_invalidation,
            Some(RuntimeDbCommitInvalidation {
                connection,
                changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
            })
        );
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        let _ = fs::remove_file(&database);
    }

    #[test]
    fn execute_runtime_value_omits_db_commit_invalidation_on_failure() {
        let database = test_path("db-commit-invalidation-failure");
        let connection = RuntimeDbConnection {
            database: database.to_string_lossy().into_owned().into_boxed_str(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let outcome = execute_runtime_value_with_effects(
            RuntimeValue::DbTask(RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
                connection,
                statements: vec![RuntimeDbStatement {
                    sql: "insert into missing_table(id) values (?)".into(),
                    arguments: vec![RuntimeValue::Int(7)],
                }],
                changed_tables: BTreeSet::from(["users".into()]),
            })),
            &mut stdout,
            &mut stderr,
        )
        .expect("db commit task should return a result value");

        let RuntimeValue::ResultErr(error) = outcome.value else {
            panic!(
                "expected failing db commit result, found {:?}",
                outcome.value
            );
        };
        let RuntimeValue::Text(message) = error.as_ref() else {
            panic!("expected failing db commit error text, found {error:?}");
        };
        assert!(
            message.contains("no such table"),
            "expected missing-table failure text, found {message}"
        );
        assert_eq!(outcome.commit_invalidation, None);
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        let _ = fs::remove_file(&database);
    }

    #[test]
    fn execute_runtime_task_plan_calls_dbus_methods_and_decodes_reply_values() {
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
            return;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let value = execute_runtime_task_plan(
            RuntimeTaskPlan::DbusCall {
                destination: "org.freedesktop.DBus".into(),
                path: "/org/freedesktop/DBus".into(),
                interface: "org.freedesktop.DBus".into(),
                member: "ListNames".into(),
                body: Box::new([]),
                bus: "session".into(),
                address: "".into(),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("D-Bus call should execute");

        let RuntimeValue::List(reply) = value else {
            panic!("D-Bus call should decode to a list of DbusValue arguments");
        };
        let [RuntimeValue::Sum(names)] = reply.as_slice() else {
            panic!("ListNames should return one D-Bus argument");
        };
        assert_eq!(names.variant_name.as_ref(), "DbusList");
        let [RuntimeValue::List(entries)] = names.fields.as_slice() else {
            panic!("ListNames should decode as DbusList payload");
        };
        assert!(
            entries.iter().any(|entry| {
                matches!(
                    entry,
                    RuntimeValue::Sum(sum)
                        if sum.variant_name.as_ref() == "DbusString"
                            && matches!(sum.fields.as_slice(), [RuntimeValue::Text(text)] if text.as_ref() == "org.freedesktop.DBus")
                )
            }),
            "reply should include org.freedesktop.DBus in returned names"
        );
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_surfaces_dbus_errors_as_dbuserror_values() {
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
            return;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = execute_runtime_task_plan(
            RuntimeTaskPlan::DbusCall {
                destination: "org.aivi.DoesNotExist".into(),
                path: "/org/aivi/DoesNotExist".into(),
                interface: "org.aivi.DoesNotExist".into(),
                member: "Ping".into(),
                body: Box::new([]),
                bus: "session".into(),
                address: "".into(),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect_err("unknown D-Bus services should fail");

        assert!(
            error.to_string().starts_with("ServiceUnknown ")
                || error.to_string().starts_with("NameNotOwned ")
                || error.to_string().starts_with("DbusProtocolError "),
            "expected structured D-Bus error, found {}",
            error
        );
    }

    #[test]
    fn execute_runtime_task_plan_runs_pkce_browser_flow() {
        let _guard = auth_test_lock();
        let token_listener = TcpListener::bind("127.0.0.1:0").expect("token listener should bind");
        let token_port = token_listener
            .local_addr()
            .expect("token listener addr should resolve")
            .port();
        let token_thread = thread::spawn(move || {
            let (mut stream, _) = token_listener
                .accept()
                .expect("token request should connect");
            let request = read_http_request(&mut stream);
            let body = request
                .split("\r\n\r\n")
                .nth(1)
                .expect("token request should contain a body");
            assert!(
                body.contains("grant_type=authorization_code"),
                "expected authorization_code grant, found {body}"
            );
            assert!(
                body.contains("code=test-code"),
                "expected test code, found {body}"
            );
            assert!(
                body.contains("code_verifier="),
                "expected code verifier in token request, found {body}"
            );
            let payload =
                r#"{"access_token":"access-123","refresh_token":"refresh-456","expires_in":3600}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            )
            .expect("token response should write");
        });

        let redirect_probe = TcpListener::bind("127.0.0.1:0").expect("redirect probe should bind");
        let redirect_port = redirect_probe
            .local_addr()
            .expect("redirect probe addr should resolve")
            .port();
        drop(redirect_probe);

        fn mock_browser_callback(url: &str) -> Result<(), String> {
            let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
            let redirect_uri = parsed
                .query_pairs()
                .find_map(|(key, value)| (key == "redirect_uri").then(|| value.into_owned()))
                .ok_or_else(|| "missing redirect_uri".to_owned())?;
            let state = parsed
                .query_pairs()
                .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
                .ok_or_else(|| "missing state".to_owned())?;
            let callback = format!("{redirect_uri}?code=test-code&state={state}");
            thread::spawn(move || {
                let _ = ureq::get(&callback).call();
            });
            Ok(())
        }
        super::set_test_browser_opener(Some(mock_browser_callback));
        struct TestBrowserOpenerGuard;
        impl Drop for TestBrowserOpenerGuard {
            fn drop(&mut self) {
                super::set_test_browser_opener(None);
            }
        }
        let _opener_guard = TestBrowserOpenerGuard;
        let token_endpoint = format!("http://127.0.0.1:{token_port}/token");
        let config = pkce_config(
            "https://auth.example/authorize",
            &token_endpoint,
            redirect_port,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let value = execute_runtime_task_plan(
            RuntimeTaskPlan::AuthPkce {
                config: Box::new(config),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("PKCE flow should succeed");
        token_thread.join().expect("token thread should finish");

        let RuntimeValue::Record(fields) = value else {
            panic!("PKCE flow should return token record");
        };
        assert!(matches!(
            super::runtime_record_field(&fields, "accessToken"),
            Some(RuntimeValue::Text(token)) if token.as_ref() == "access-123"
        ));
        assert!(matches!(
            super::runtime_record_field(&fields, "refreshToken"),
            Some(RuntimeValue::OptionSome(inner))
                if matches!(inner.as_ref(), RuntimeValue::Text(token) if token.as_ref() == "refresh-456")
        ));
        assert!(matches!(
            super::runtime_record_field(&fields, "expiresAt"),
            Some(RuntimeValue::OptionSome(inner)) if matches!(inner.as_ref(), RuntimeValue::Int(_))
        ));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_runtime_task_plan_refreshes_pkce_tokens() {
        let token_listener = TcpListener::bind("127.0.0.1:0").expect("token listener should bind");
        let token_port = token_listener
            .local_addr()
            .expect("token listener addr should resolve")
            .port();
        let token_thread = thread::spawn(move || {
            let (mut stream, _) = token_listener
                .accept()
                .expect("refresh request should connect");
            let mut buffer = [0u8; 4096];
            let read = stream
                .read(&mut buffer)
                .expect("refresh request should read");
            let request = String::from_utf8_lossy(&buffer[..read]);
            let body = request
                .split("\r\n\r\n")
                .nth(1)
                .expect("refresh request should contain a body");
            assert!(
                body.contains("grant_type=refresh_token"),
                "expected refresh grant, found {body}"
            );
            assert!(
                body.contains("refresh_token=refresh-old"),
                "expected refresh token in request, found {body}"
            );
            let payload = r#"{"access_token":"access-new","expires_in":1800}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            )
            .expect("refresh response should write");
        });

        let config = pkce_config(
            "https://auth.example/authorize",
            &format!("http://127.0.0.1:{token_port}/token"),
            43123,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let value = execute_runtime_task_plan(
            RuntimeTaskPlan::AuthRefresh {
                config: Box::new(config),
                refresh_token: "refresh-old".into(),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("refresh flow should succeed");
        token_thread.join().expect("refresh thread should finish");

        let RuntimeValue::Record(fields) = value else {
            panic!("refresh flow should return token record");
        };
        assert!(matches!(
            super::runtime_record_field(&fields, "accessToken"),
            Some(RuntimeValue::Text(token)) if token.as_ref() == "access-new"
        ));
        assert!(matches!(
            super::runtime_record_field(&fields, "refreshToken"),
            Some(RuntimeValue::OptionSome(inner))
                if matches!(inner.as_ref(), RuntimeValue::Text(token) if token.as_ref() == "refresh-old")
        ));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }
}
