use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

use aivi_backend::{
    RuntimeDbCommitPlan, RuntimeDbQueryPlan, RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeMap,
    RuntimeMapEntry, RuntimeTaskPlan, RuntimeValue,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTaskExecutionError {
    message: Box<str>,
}

impl RuntimeTaskExecutionError {
    fn new(message: impl Into<String>) -> Self {
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
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    execute_runtime_task_plan(plan, &mut stdout, &mut stderr)
}

pub fn execute_runtime_value(
    value: RuntimeValue,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_effects(value, stdout, stderr)?.value)
}

pub(crate) fn execute_runtime_value_with_effects(
    value: RuntimeValue,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    match value {
        RuntimeValue::Task(plan) => {
            execute_runtime_task_plan(plan, stdout, stderr).map(RuntimeTaskExecutionOutcome::value)
        }
        RuntimeValue::DbTask(plan) => execute_runtime_db_task_plan_with_effects(plan),
        other => Ok(RuntimeTaskExecutionOutcome::value(other)),
    }
}

pub fn execute_runtime_value_with_stdio(
    value: RuntimeValue,
) -> Result<RuntimeValue, RuntimeTaskExecutionError> {
    Ok(execute_runtime_value_with_stdio_effects(value)?.value)
}

pub(crate) fn execute_runtime_value_with_stdio_effects(
    value: RuntimeValue,
) -> Result<RuntimeTaskExecutionOutcome, RuntimeTaskExecutionError> {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    execute_runtime_value_with_effects(value, &mut stdout, &mut stderr)
}

fn task_error(message: impl Into<String>) -> RuntimeTaskExecutionError {
    RuntimeTaskExecutionError::new(message)
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
    use std::{collections::BTreeSet, fs, path::PathBuf};

    use aivi_backend::{
        RuntimeDbCommitPlan, RuntimeDbConnection, RuntimeDbQueryPlan, RuntimeDbStatement,
        RuntimeDbTaskPlan, RuntimeMap, RuntimeMapEntry, RuntimeTaskPlan, RuntimeValue,
    };

    use super::{
        RuntimeDbCommitInvalidation, execute_runtime_task_plan, execute_runtime_value,
        execute_runtime_value_with_effects,
    };

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
}
