
/// Return an XDG base directory: use the env var if set and non-empty, otherwise `$HOME/fallback`.
fn xdg_dir(env_var: &str, fallback: &str) -> String {
    if let Ok(val) = std::env::var(env_var)
        && !val.is_empty() {
            return val;
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
                    reason: "floor result is not finite",
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
                    reason: "ceil result is not finite",
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
                    reason: "round result is not finite",
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
                    reason: "abs result is not finite",
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
                    reason: "int-to-float result is not finite",
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
            combined.extend_from_slice(&right);
            Ok(RuntimeValue::Bytes(combined.into()))
        }
        (IntrinsicValue::BytesFromText, [t]) => {
            let text = expect_intrinsic_text(kernel, expr, value, 0, t)?;
            Ok(RuntimeValue::Bytes(text.as_bytes().into()))
        }
        (IntrinsicValue::BytesToText, [b]) => {
            let bytes = expect_intrinsic_bytes(kernel, expr, value, 0, b)?;
            Ok(std::str::from_utf8(&bytes)
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

