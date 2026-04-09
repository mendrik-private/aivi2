fn dbus_value_step_supported(
    decode: &hir::SourceDecodeProgram,
    step: hir::DecodeProgramStepId,
    visiting: &mut HashSet<hir::DecodeProgramStepId>,
) -> bool {
    if !visiting.insert(step) {
        return true;
    }
    let result = match decode.step(step) {
        hir::DecodeProgramStep::Sum { variants, .. } => {
            variants
                .iter()
                .all(|variant| match (variant.name.as_str(), variant.payload) {
                    ("DbusString", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Text,
                        }
                    ),
                    ("DbusInt", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Int,
                        }
                    ),
                    ("DbusBool", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Bool,
                        }
                    ),
                    ("DbusList", Some(payload)) | ("DbusStruct", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::List { element }
                            if dbus_value_step_supported(decode, *element, visiting)
                    ),
                    ("DbusVariant", Some(payload)) => {
                        dbus_value_step_supported(decode, payload, visiting)
                    }
                    _ => false,
                })
        }
        _ => false,
    };
    visiting.remove(&step);
    result
}

const MAX_DBUS_VALUE_DEPTH: usize = 64;

fn dbus_body_external(parameters: Option<&Variant>) -> Result<ExternalSourceValue, Box<str>> {
    let Some(parameters) = parameters else {
        return Ok(ExternalSourceValue::List(Vec::new()));
    };
    let values = if parameters.type_().is_tuple() {
        (0..parameters.n_children())
            .map(|index| dbus_value_external(&parameters.child_value(index), 0))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![dbus_value_external(parameters, 0)?]
    };
    Ok(ExternalSourceValue::List(values))
}

fn dbus_value_external(value: &Variant, depth: usize) -> Result<ExternalSourceValue, Box<str>> {
    if depth >= MAX_DBUS_VALUE_DEPTH {
        return Err("D-Bus payload nesting exceeds the current runtime depth limit".into());
    }
    match value.classify() {
        VariantClass::Boolean => Ok(ExternalSourceValue::variant_with_payload(
            "DbusBool",
            ExternalSourceValue::Bool(
                value
                    .get::<bool>()
                    .ok_or_else(|| "failed to decode D-Bus boolean payload".to_owned())?,
            ),
        )),
        VariantClass::Byte => Ok(ExternalSourceValue::variant_with_payload(
            "DbusInt",
            ExternalSourceValue::Int(
                value
                    .get::<u8>()
                    .ok_or_else(|| "failed to decode D-Bus byte payload".to_owned())?
                    as i64,
            ),
        )),
        VariantClass::Int16 => dbus_int_value(
            value
                .get::<i16>()
                .ok_or_else(|| "failed to decode D-Bus int16 payload".to_owned())?
                as i64,
        ),
        VariantClass::Uint16 => dbus_int_value(
            value
                .get::<u16>()
                .ok_or_else(|| "failed to decode D-Bus uint16 payload".to_owned())?
                as i64,
        ),
        VariantClass::Int32 => dbus_int_value(
            value
                .get::<i32>()
                .ok_or_else(|| "failed to decode D-Bus int32 payload".to_owned())?
                as i64,
        ),
        VariantClass::Uint32 => dbus_int_value(
            value
                .get::<u32>()
                .ok_or_else(|| "failed to decode D-Bus uint32 payload".to_owned())?
                as i64,
        ),
        VariantClass::Int64 => dbus_int_value(
            value
                .get::<i64>()
                .ok_or_else(|| "failed to decode D-Bus int64 payload".to_owned())?,
        ),
        VariantClass::Uint64 => {
            let value = value
                .get::<u64>()
                .ok_or_else(|| "failed to decode D-Bus uint64 payload".to_owned())?;
            let value = i64::try_from(value)
                .map_err(|_| "D-Bus uint64 payload exceeds the current Int runtime slice")?;
            dbus_int_value(value)
        }
        VariantClass::Handle => dbus_int_value(
            value
                .get::<i32>()
                .ok_or_else(|| "failed to decode D-Bus handle payload".to_owned())?
                as i64,
        ),
        VariantClass::String | VariantClass::ObjectPath | VariantClass::Signature => {
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusString",
                ExternalSourceValue::Text(
                    value
                        .str()
                        .ok_or_else(|| "failed to decode D-Bus string payload".to_owned())?
                        .into(),
                ),
            ))
        }
        VariantClass::Variant => {
            let inner = value
                .as_variant()
                .ok_or_else(|| "failed to decode nested D-Bus variant payload".to_owned())?;
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusVariant",
                dbus_value_external(&inner, depth + 1)?,
            ))
        }
        VariantClass::Array => {
            let mut values = Vec::with_capacity(value.n_children());
            for index in 0..value.n_children() {
                values.push(dbus_value_external(&value.child_value(index), depth + 1)?);
            }
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusList",
                ExternalSourceValue::List(values),
            ))
        }
        VariantClass::Tuple | VariantClass::DictEntry => {
            let mut values = Vec::with_capacity(value.n_children());
            for index in 0..value.n_children() {
                values.push(dbus_value_external(&value.child_value(index), depth + 1)?);
            }
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusStruct",
                ExternalSourceValue::List(values),
            ))
        }
        VariantClass::Maybe => Err(
            "D-Bus maybe payloads are not representable by the current DbusValue runtime slice"
                .into(),
        ),
        VariantClass::Double => Err(
            "D-Bus floating-point payloads are not representable by the current DbusValue runtime slice"
                .into(),
        ),
        VariantClass::__Unknown(_) => Err("unknown D-Bus payload class".into()),
        _ => Err("unsupported D-Bus payload class".into()),
    }
}

fn dbus_int_value(value: i64) -> Result<ExternalSourceValue, Box<str>> {
    Ok(ExternalSourceValue::variant_with_payload(
        "DbusInt",
        ExternalSourceValue::Int(value),
    ))
}

fn open_dbus_connection(bus: DbusBus, address: Option<&str>) -> Result<DBusConnection, Box<str>> {
    match address {
        Some(address) => DBusConnection::for_address_sync(
            address,
            DBusConnectionFlags::AUTHENTICATION_CLIENT
                | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None::<&gio::DBusAuthObserver>,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string().into_boxed_str()),
        None => gio::bus_get_sync(bus.bus_type(), None::<&gio::Cancellable>)
            .map_err(|error| error.to_string().into_boxed_str()),
    }
}

fn spawn_dbus_own_name_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusOwnNamePlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusOwnName;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let owned_port = port.clone();
            let owned_output = plan.output.clone();
            let lost_port = port.clone();
            let lost_output = plan.output.clone();
            let owned_connection = plan
                .address
                .as_deref()
                .map(|address| open_dbus_connection(plan.bus, Some(address)))
                .transpose()?;
            let owner_id = if let Some(connection) = owned_connection.as_ref() {
                gio::bus_own_name_on_connection(
                    connection,
                    plan.name.as_ref(),
                    plan.flags,
                    move |_, _| {
                        if let Ok(value) = owned_output.value_for_state("Owned") {
                            let _ =
                                owned_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = lost_output.value_for_state("Lost") {
                            let _ =
                                lost_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                )
            } else {
                let queued_port = port.clone();
                let queued_output = plan.output.clone();
                let queue_enabled = !plan.flags.contains(BusNameOwnerFlags::DO_NOT_QUEUE);
                gio::bus_own_name(
                    plan.bus.bus_type(),
                    plan.name.as_ref(),
                    plan.flags,
                    move |_, _| {
                        if queue_enabled && let Ok(value) = queued_output.value_for_state("Queued")
                        {
                            let _ = queued_port
                                .publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = owned_output.value_for_state("Owned") {
                            let _ =
                                owned_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = lost_output.value_for_state("Lost") {
                            let _ =
                                lost_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                )
            };
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            gio::bus_unown_name(owner_id);
            drop(owned_connection);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn spawn_dbus_signal_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusSignalPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusSignal;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = open_dbus_connection(plan.bus, plan.address.as_deref())?;
            let output = plan.output.clone();
            let publish_port = port.clone();
            #[allow(deprecated)]
            let subscription_id = connection.signal_subscribe(
                None,
                plan.interface.as_deref(),
                plan.member.as_deref(),
                Some(plan.path.as_ref()),
                None,
                DBusSignalFlags::NONE,
                move |_, _, object_path, interface_name, signal_name, parameters| {
                    let Ok(value) =
                        output.signal_value(object_path, interface_name, signal_name, parameters)
                    else {
                        return;
                    };
                    let _ = publish_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                },
            );
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            #[allow(deprecated)]
            connection.signal_unsubscribe(subscription_id);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn spawn_dbus_method_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusMethodPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusMethod;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = open_dbus_connection(plan.bus, plan.address.as_deref())?;
            let reply_variant = plan
                .reply_body
                .as_deref()
                .map(|text| {
                    Variant::parse(None, text).map_err(|err| {
                        format!("dbus.method reply option is not a valid GLib variant: {err}")
                            .into_boxed_str()
                    })
                })
                .transpose()?;
            let output = plan.output.clone();
            let publish_port = port.clone();
            let destination = plan.destination.clone();
            let path = plan.path.clone();
            let interface = plan.interface.clone();
            let member = plan.member.clone();
            let filter_id = connection.add_filter(move |connection, message, incoming| {
                if !incoming
                    || message.message_type() != DBusMessageType::MethodCall
                    || message.destination().as_deref() != Some(destination.as_ref())
                    || path
                        .as_deref()
                        .is_some_and(|expected| message.path().as_deref() != Some(expected))
                    || interface
                        .as_deref()
                        .is_some_and(|expected| message.interface().as_deref() != Some(expected))
                    || member
                        .as_deref()
                        .is_some_and(|expected| message.member().as_deref() != Some(expected))
                {
                    return Some(message.clone());
                }
                let reply = message.new_method_reply();
                if let Some(body) = &reply_variant {
                    reply.set_body(body);
                }
                let _ = connection.send_message(&reply, DBusSendMessageFlags::NONE);
                if let (Some(path), Some(interface), Some(member)) =
                    (message.path(), message.interface(), message.member())
                    && let Ok(value) = output.method_value(
                        destination.as_ref(),
                        path.as_str(),
                        interface.as_str(),
                        member.as_str(),
                        message.body().as_ref(),
                    )
                {
                    let _ = publish_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
                None
            });
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            connection.remove_filter(filter_id);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn install_dbus_stop_timer(
    main_loop: &MainLoop,
    stop: &Arc<AtomicBool>,
    port: &DetachedRuntimePublicationPort,
) {
    let main_loop = main_loop.clone();
    let stop = stop.clone();
    let port = port.clone();
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            main_loop.quit();
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
}

fn finish_dbus_startup(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    handle: thread::JoinHandle<()>,
    startup_rx: mpsc::Receiver<Result<(), Box<str>>>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    match startup_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(detail)) => {
            let _ = handle.join();
            Err(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail,
            })
        }
        Err(error) => {
            let _ = handle.join();
            Err(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("failed to receive provider startup status: {error}")
                    .into_boxed_str(),
            })
        }
    }
}

fn spawn_db_connect_worker(
    instance: SourceInstanceId,
    port: DetachedRuntimePublicationPort,
    plan: DbConnectPlan,
    context: SourceProviderContext,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let Ok(value) = execute_db_connect(instance, &plan) else {
            return;
        };
        let Ok(value) = execute_runtime_value_with_context_with_stdio(value, &context) else {
            return;
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    })
}

fn spawn_db_live_worker(
    instance: SourceInstanceId,
    port: DetachedRuntimePublicationPort,
    plan: DbLivePlan,
    context: SourceProviderContext,
    delay: Duration,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        if !delay.is_zero() && sleep_with_cancellation(delay, &port) {
            return;
        }
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let value = match execute_runtime_value_with_context_with_stdio(plan.task.clone(), &context)
        {
            Ok(value) => value,
            Err(error) => {
                let Some(result) = &plan.result else {
                    return;
                };
                let Ok(value) = db_live_query_error_value(instance, result, &error.to_string())
                else {
                    return;
                };
                value
            }
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    })
}

fn spawn_timer_every(
    port: DetachedRuntimePublicationPort,
    plan: TimerPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if plan.immediate && port.publish(DetachedRuntimeValue::unit()).is_err() {
            return;
        }
        let mut next_tick = Instant::now() + plan.delay;
        while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
            let sleep_dur = match plan.jitter {
                Some(jitter) => {
                    let jitter_nanos = jitter.as_nanos() as u64;
                    let offset = if jitter_nanos > 0 {
                        Duration::from_nanos(fastrand::u64(0..=jitter_nanos))
                    } else {
                        Duration::ZERO
                    };
                    plan.delay + offset
                }
                None => plan.delay,
            };
            thread::sleep(sleep_dur);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                break;
            }
            if plan.coalesce {
                // Coalescing: fire exactly once per sleep cycle.
                if port.publish(DetachedRuntimeValue::unit()).is_err() {
                    break;
                }
            } else {
                // Non-coalescing: fire all ticks that are due since the last cycle.
                let now = Instant::now();
                while next_tick <= now {
                    if port.publish(DetachedRuntimeValue::unit()).is_err() {
                        return;
                    }
                    next_tick += plan.delay;
                }
            }
        }
    })
}

fn spawn_timer_after(
    port: DetachedRuntimePublicationPort,
    plan: TimerPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if !plan.immediate {
            thread::sleep(plan.delay);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
        }
        let _ = port.publish(DetachedRuntimeValue::unit());
    })
}

fn spawn_http_worker(
    port: DetachedRuntimePublicationPort,
    plan: HttpPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
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
    })
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

fn spawn_fs_read_worker(
    port: DetachedRuntimePublicationPort,
    plan: FsReadPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
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
    })
}

fn spawn_fs_watch_worker(
    port: DetachedRuntimePublicationPort,
    plan: FsWatchPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if plan.recursive {
            let mut previous = dir_signatures(&plan.path);
            while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
                thread::sleep(Duration::from_millis(40));
                if stop.load(Ordering::Acquire) || port.is_cancelled() {
                    break;
                }
                let current = dir_signatures(&plan.path);
                // Detect created/changed/deleted entries by comparing the two snapshots.
                for (path, sig) in &current {
                    match previous.get(path) {
                        None => {
                            if emit_fs_event("Created", &plan, &port).is_err() {
                                return;
                            }
                        }
                        Some(prev) if prev != sig => {
                            if emit_fs_event("Changed", &plan, &port).is_err() {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                for path in previous.keys() {
                    if !current.contains_key(path) {
                        if emit_fs_event("Deleted", &plan, &port).is_err() {
                            return;
                        }
                    }
                }
                previous = current;
            }
        } else {
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
                if emit_fs_event(event, &plan, &port).is_err() {
                    return;
                }
            }
        }
    })
}

fn emit_fs_event(
    event: &str,
    plan: &FsWatchPlan,
    port: &DetachedRuntimePublicationPort,
) -> Result<(), ()> {
    if !plan.events.contains(event) {
        return Ok(());
    }
    let Ok(Some(value)) = plan.output.value_for_name(event) else {
        return Ok(());
    };
    port.publish(DetachedRuntimeValue::from_runtime_owned(value))
        .map_err(|_| ())
}

/// Collect file signatures for all entries in a directory tree.
fn dir_signatures(root: &Path) -> BTreeMap<PathBuf, FileSignature> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let sig = file_signature(&path);
                map.insert(path, sig);
            }
        }
    }
    map
}

fn spawn_socket_worker(
    port: DetachedRuntimePublicationPort,
    plan: SocketPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            match TcpStream::connect((plan.host.as_ref(), plan.port)) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    // When heartbeat is configured, spawn a keepalive writer thread that
                    // periodically sends an empty byte to prevent idle timeouts.
                    let heartbeat_stop = stop.clone();
                    let heartbeat_cancel = port.cancellation();
                    let heartbeat_handle = plan.heartbeat.map(|interval| {
                        let mut writer =
                            stream.try_clone().expect("TcpStream clone should succeed");
                        thread::spawn(move || {
                            use std::io::Write;
                            while !heartbeat_stop.load(Ordering::Acquire)
                                && !heartbeat_cancel.is_cancelled()
                            {
                                thread::sleep(interval);
                                if heartbeat_stop.load(Ordering::Acquire)
                                    || heartbeat_cancel.is_cancelled()
                                {
                                    break;
                                }
                                // Send a single newline as a keepalive ping.
                                if writer.write_all(b"\n").is_err() || writer.flush().is_err() {
                                    break;
                                }
                            }
                        })
                    });
                    let mut reader = BufReader::with_capacity(plan.buffer.max(1), stream);
                    let mut line = String::new();
                    loop {
                        if stop.load(Ordering::Acquire) || port.is_cancelled() {
                            if let Some(h) = heartbeat_handle {
                                let _ = h.join();
                            }
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
                                    if let Some(h) = heartbeat_handle {
                                        let _ = h.join();
                                    }
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
                    if let Some(h) = heartbeat_handle {
                        let _ = h.join();
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
            if !plan.reconnect
                || stop.load(Ordering::Acquire)
                || sleep_with_cancellation(Duration::from_millis(100), &port)
            {
                return;
            }
        }
    })
}

fn spawn_mailbox_worker(
    port: DetachedRuntimePublicationPort,
    plan: MailboxPlan,
    receiver: mpsc::Receiver<Box<str>>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_heartbeat = Instant::now();
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            // Check if a heartbeat ping is due.
            if let Some(interval) = plan.heartbeat {
                if last_heartbeat.elapsed() >= interval {
                    last_heartbeat = Instant::now();
                    if port.publish(DetachedRuntimeValue::unit()).is_err() {
                        return;
                    }
                }
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
                    if plan.reconnect {
                        // Wait briefly and continue; the sender side may re-establish.
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
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
    })
}

fn spawn_process_worker(
    port: DetachedRuntimePublicationPort,
    plan: ProcessPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
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
            let events = plan.events.clone();
            let bytes_mode = plan.stdout_mode == ProcessStreamMode::Bytes;
            thread::spawn(move || {
                if bytes_mode {
                    read_process_stream_bytes(stdout, port, events, true)
                } else {
                    read_process_stream(stdout, port, events, true)
                }
            })
        });
        let stderr_handle = child.stderr.take().map(|stderr| {
            let port = port.clone();
            let events = plan.events.clone();
            let bytes_mode = plan.stderr_mode == ProcessStreamMode::Bytes;
            thread::spawn(move || {
                if bytes_mode {
                    read_process_stream_bytes(stderr, port, events, false)
                } else {
                    read_process_stream(stderr, port, events, false)
                }
            })
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
    })
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

fn read_process_stream_bytes(
    stream: impl std::io::Read,
    port: DetachedRuntimePublicationPort,
    plan: ProcessEventPlan,
    stdout: bool,
) {
    let mut reader = BufReader::new(stream);
    let mut buf = vec![0u8; 4096];
    while !port.is_cancelled() {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = buf[..n].to_vec().into_boxed_slice();
                let value = if stdout {
                    plan.stdout_bytes_value(chunk)
                } else {
                    plan.stderr_bytes_value(chunk)
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
        BuiltinSourceProvider::ApiGet => "GET",
        BuiltinSourceProvider::ApiPost => "POST",
        BuiltinSourceProvider::ApiPut => "PUT",
        BuiltinSourceProvider::ApiPatch => "PATCH",
        BuiltinSourceProvider::ApiDelete => "DELETE",
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

fn file_signature(path: &Path) -> FileSignature {
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

fn parse_positive_int(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<i64, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value > 0 => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "positive Int".into(),
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

fn parse_task_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Task(task) => Ok(RuntimeValue::Task(task.clone())),
        RuntimeValue::DbTask(task) => Ok(RuntimeValue::DbTask(task.clone())),
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Task or DbTask".into(),
            value: other.clone(),
        }),
    }
}

fn parse_db_connect_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    context: &SourceProviderContext,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    let database = match strip_detached_signal(value) {
        RuntimeValue::Text(value) => value.clone(),
        RuntimeValue::Record(fields) => {
            let Some(field) = fields
                .iter()
                .find(|field| field.label.as_ref() == "database")
            else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Text or { database: Text }".into(),
                    value: strip_detached_signal(value).clone(),
                });
            };
            let RuntimeValue::Text(database) = strip_signal(&field.value) else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Text or { database: Text }".into(),
                    value: strip_detached_signal(value).clone(),
                });
            };
            database.clone()
        }
        other => {
            return Err(SourceProviderExecutionError::InvalidArgument {
                instance,
                provider,
                index,
                expected: "Text or { database: Text }".into(),
                value: other.clone(),
            });
        }
    };
    Ok(context.normalize_sqlite_database_text(database.as_ref()))
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
        .map(|(k, v)| {
            let RuntimeValue::Text(key) = strip_signal(k) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(k).clone(),
                });
            };
            let RuntimeValue::Text(value) = strip_signal(v) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(v).clone(),
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

fn validate_argument_count(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    config: &EvaluatedSourceConfig,
    expected: usize,
) -> Result<(), SourceProviderExecutionError> {
    if config.arguments.len() != expected {
        return Err(SourceProviderExecutionError::InvalidArgumentCount {
            instance,
            provider,
            expected,
            found: config.arguments.len(),
        });
    }
    Ok(())
}

fn reject_options(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    config: &EvaluatedSourceConfig,
) -> Result<(), SourceProviderExecutionError> {
    if let Some(option) = config.options.first() {
        return Err(SourceProviderExecutionError::UnsupportedOption {
            instance,
            provider,
            option_name: option.option_name.clone(),
        });
    }
    Ok(())
}

fn publish_immediate_value(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    port: &DetachedRuntimePublicationPort,
    value: RuntimeValue,
) -> Result<(), SourceProviderExecutionError> {
    port.publish(DetachedRuntimeValue::from_runtime_owned(value))
        .map_err(|error| SourceProviderExecutionError::StartFailed {
            instance,
            provider,
            detail: format!("failed to publish initial value: {error:?}").into_boxed_str(),
        })
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
        "Bytes" => Ok(ProcessStreamMode::Bytes),
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

fn execute_db_connect(
    instance: SourceInstanceId,
    plan: &DbConnectPlan,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let output = match Command::new("sqlite3")
        .arg(plan.database.as_ref())
        .arg("PRAGMA schema_version;")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return db_connect_error_value(
                instance,
                &plan.result,
                &format!("failed to start sqlite3: {error}"),
            );
        }
    };
    if output.status.success() {
        db_connect_success_value(instance, &plan.result, &plan.database)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            format!("sqlite3 exited with status {}", output.status)
        } else {
            stderr
        };
        db_connect_error_value(instance, &plan.result, &detail)
    }
}

fn db_connect_success_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    database: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbConnect;
    let payload = serde_json::json!({
        "database": database,
    });
    let encoded = serde_json::to_string(&payload).expect("db.connect payload should encode");
    result.success_from_text(&encoded).map_err(|error| {
        SourceProviderExecutionError::UnsupportedProviderShape {
            instance,
            provider,
            detail: format!(
                "db.connect success payload does not match the source output shape: {error}"
            )
            .into_boxed_str(),
        }
    })
}

fn db_connect_error_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    detail: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbConnect;
    result
        .error_value(TextSourceErrorKind::Connect, detail)
        .map_err(
            |shape| SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "db.connect failure cannot be represented by the current error type: {shape}"
                )
                .into_boxed_str(),
            },
        )
}

fn db_live_query_error_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    detail: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbLive;
    result
        .error_value(TextSourceErrorKind::Query, detail)
        .map_err(
            |shape| SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "db.live failure cannot be represented by the current error type: {shape}"
                )
                .into_boxed_str(),
            },
        )
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

