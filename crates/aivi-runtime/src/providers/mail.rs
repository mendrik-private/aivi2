use std::io::Write;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use glib::prelude::ToVariant;
use mailparse::MailHeaderMap;
use native_tls::TlsConnector;

#[derive(Clone)]
struct GoaMailAccountsPlan {
    instance: SourceInstanceId,
    result: RequestResultPlan,
}

impl GoaMailAccountsPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::GoaMailAccounts;
        validate_argument_count(instance, provider, config, 0)?;
        reject_options(instance, provider, config)?;
        Ok(Self {
            instance,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct DbusEmitPlan {
    instance: SourceInstanceId,
    bus: DbusBus,
    address: Option<Box<str>>,
    path: Box<str>,
    interface: Box<str>,
    member: Box<str>,
    body: Box<str>,
    result: RequestResultPlan,
}

impl DbusEmitPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusEmit;
        validate_argument_count(instance, provider, config, 1)?;
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut interface = None;
        let mut member = None;
        let mut body: Box<str> = "".into();
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
                "body" => {
                    body = parse_text_option(instance, provider, &option.option_name, &option.value)?;
                }
                "refreshOn" | "activeWhen" => {}
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
            interface: interface.unwrap_or_else(|| "io.mailfox.Daemon".into()),
            member: member.unwrap_or_else(|| "Changed".into()),
            body,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone, Debug)]
struct ImapAccountConfig {
    account_id: Box<str>,
    host: Box<str>,
    port: u16,
    user: Box<str>,
    use_ssl: bool,
    use_tls: bool,
    auth: ImapAuthConfig,
}

#[derive(Clone, Debug)]
enum ImapAuthConfig {
    Password(Box<str>),
    OAuthToken(Box<str>),
}

#[derive(Clone)]
struct ImapConnectPlan {
    instance: SourceInstanceId,
    accounts: Vec<ImapAccountConfig>,
    mailbox: Box<str>,
    limit: usize,
    result: RequestResultPlan,
}

impl ImapConnectPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ImapConnect;
        validate_argument_count(instance, provider, config, 1)?;
        let accounts = parse_imap_accounts_argument(instance, provider, 0, &config.arguments[0])?;
        let mut mailbox: Box<str> = "INBOX".into();
        let mut limit = 25_usize;
        for option in &config.options {
            match option.option_name.as_ref() {
                "mailbox" => {
                    mailbox =
                        parse_text_option(instance, provider, &option.option_name, &option.value)?;
                }
                "limit" => {
                    limit = parse_positive_int(instance, provider, &option.option_name, &option.value)?
                        as usize;
                }
                "refreshOn" | "activeWhen" => {}
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
            accounts,
            mailbox,
            limit,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct ImapIdlePlan {
    instance: SourceInstanceId,
    accounts: Vec<ImapAccountConfig>,
    mailbox: Box<str>,
    result: RequestResultPlan,
}

impl ImapIdlePlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ImapIdle;
        validate_argument_count(instance, provider, config, 1)?;
        let accounts = parse_imap_accounts_argument(instance, provider, 0, &config.arguments[0])?;
        let mut mailbox: Box<str> = "INBOX".into();
        for option in &config.options {
            match option.option_name.as_ref() {
                "mailbox" => {
                    mailbox =
                        parse_text_option(instance, provider, &option.option_name, &option.value)?;
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
        Ok(Self {
            instance,
            accounts,
            mailbox,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct ImapFetchBodyPlan {
    instance: SourceInstanceId,
    request: ImapBodyRequest,
    result: RequestResultPlan,
}

#[derive(Clone, Debug)]
struct ImapBodyRequest {
    account: ImapAccountConfig,
    mailbox: Box<str>,
    uid: u32,
}

impl ImapFetchBodyPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ImapFetchBody;
        validate_argument_count(instance, provider, config, 1)?;
        for option in &config.options {
            match option.option_name.as_ref() {
                "refreshOn" | "activeWhen" => {}
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
            request: parse_imap_body_request(instance, provider, 0, &config.arguments[0])?,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

fn spawn_goa_mail_accounts_worker(
    port: DetachedRuntimePublicationPort,
    plan: GoaMailAccountsPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::GoaMailAccounts;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>)
                .map_err(|error| error.to_string().into_boxed_str())?;
            publish_goa_mail_accounts(&connection, &port, &plan)?;
            let publish_port = port.clone();
            let publish_plan = plan.clone();
            let subscription_added = connection.signal_subscribe(
                Some("org.gnome.OnlineAccounts"),
                Some("org.freedesktop.DBus.ObjectManager"),
                Some("InterfacesAdded"),
                None,
                None,
                DBusSignalFlags::NONE,
                move |connection, _, _, _, _, _| {
                    let _ = publish_goa_mail_accounts(connection, &publish_port, &publish_plan);
                },
            );
            let publish_port = port.clone();
            let publish_plan = plan.clone();
            let subscription_removed = connection.signal_subscribe(
                Some("org.gnome.OnlineAccounts"),
                Some("org.freedesktop.DBus.ObjectManager"),
                Some("InterfacesRemoved"),
                None,
                None,
                DBusSignalFlags::NONE,
                move |connection, _, _, _, _, _| {
                    let _ = publish_goa_mail_accounts(connection, &publish_port, &publish_plan);
                },
            );
            let publish_port = port.clone();
            let publish_plan = plan.clone();
            let subscription_changed = connection.signal_subscribe(
                Some("org.gnome.OnlineAccounts"),
                Some("org.freedesktop.DBus.Properties"),
                Some("PropertiesChanged"),
                None,
                None,
                DBusSignalFlags::NONE,
                move |connection, _, _, _, _, _| {
                    let _ = publish_goa_mail_accounts(connection, &publish_port, &publish_plan);
                },
            );
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            #[allow(deprecated)]
            {
                connection.signal_unsubscribe(subscription_added);
                connection.signal_unsubscribe(subscription_removed);
                connection.signal_unsubscribe(subscription_changed);
            }
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

fn spawn_dbus_emit_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusEmitPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusEmit;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = open_dbus_connection(plan.bus, plan.address.as_deref())?;
            let payload = (!plan.body.is_empty())
                .then(|| Variant::tuple_from_iter([plan.body.as_ref().to_variant()]));
            connection
                .emit_signal(
                    None,
                    plan.path.as_ref(),
                    plan.interface.as_ref(),
                    plan.member.as_ref(),
                    payload.as_ref(),
                )
                .map_err(|error| error.to_string().into_boxed_str())?;
            let value = decode_ok_external(
                plan.instance,
                provider,
                &plan.result.decode,
                ExternalSourceValue::Unit,
            )?;
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
            let _ = startup_tx.send(Ok(()));
            main_loop.quit();
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

fn spawn_imap_connect_worker(
    port: DetachedRuntimePublicationPort,
    plan: ImapConnectPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    Ok(thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let value = match imap_connect_value(&plan) {
            Ok(value) => value,
            Err(detail) => match decode_err_external(
                plan.instance,
                BuiltinSourceProvider::ImapConnect,
                &plan.result.decode,
                imap_error_external(&detail),
            ) {
                Ok(value) => value,
                Err(_) => return,
            },
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    }))
}

fn spawn_imap_idle_worker(
    port: DetachedRuntimePublicationPort,
    plan: ImapIdlePlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    Ok(thread::spawn(move || {
        let mut last_seen = BTreeMap::<Box<str>, u32>::new();
        while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
            for account in &plan.accounts {
                if stop.load(Ordering::Acquire) || port.is_cancelled() {
                    return;
                }
                match imap_highest_uid(account, plan.mailbox.as_ref()) {
                    Ok(Some(uid)) => {
                        let previous = last_seen.insert(account.account_id.clone(), uid);
                        if let Some(previous) = previous && uid > previous {
                            let payload = ExternalSourceValue::Record(BTreeMap::from([
                                ("accountId".into(), ExternalSourceValue::Text(account.account_id.clone())),
                                ("mailbox".into(), ExternalSourceValue::Text(plan.mailbox.clone())),
                                (
                                    "event".into(),
                                    ExternalSourceValue::variant_with_payload(
                                        "NewMessage",
                                        ExternalSourceValue::Int(uid as i64),
                                    ),
                                ),
                            ]));
                            let Ok(value) = decode_ok_external(
                                plan.instance,
                                BuiltinSourceProvider::ImapIdle,
                                &plan.result.decode,
                                payload,
                            ) else {
                                return;
                            };
                            if port
                                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(detail) => {
                        let Ok(value) = decode_err_external(
                            plan.instance,
                            BuiltinSourceProvider::ImapIdle,
                            &plan.result.decode,
                            imap_error_external(&detail),
                        ) else {
                            return;
                        };
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                }
            }
            if sleep_with_cancellation(Duration::from_secs(30), &port)
                || stop.load(Ordering::Acquire)
            {
                return;
            }
        }
    }))
}

fn spawn_imap_fetch_body_worker(
    port: DetachedRuntimePublicationPort,
    plan: ImapFetchBodyPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    Ok(thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let value = match imap_body_value(&plan) {
            Ok(value) => value,
            Err(detail) => match decode_err_external(
                plan.instance,
                BuiltinSourceProvider::ImapFetchBody,
                &plan.result.decode,
                imap_error_external(&detail),
            ) {
                Ok(value) => value,
                Err(_) => return,
            },
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    }))
}

fn publish_goa_mail_accounts(
    connection: &DBusConnection,
    port: &DetachedRuntimePublicationPort,
    plan: &GoaMailAccountsPlan,
) -> Result<(), Box<str>> {
    let payload = goa_mail_accounts_external(connection)?;
    let value = decode_ok_external(
        plan.instance,
        BuiltinSourceProvider::GoaMailAccounts,
        &plan.result.decode,
        payload,
    )?;
    port.publish(DetachedRuntimeValue::from_runtime_owned(value))
        .map_err(|_| "provider publication port closed before GOA update could publish".into())
}

fn goa_mail_accounts_external(connection: &DBusConnection) -> Result<ExternalSourceValue, Box<str>> {
    let reply = connection
        .call_sync(
            Some("org.gnome.OnlineAccounts"),
            "/org/gnome/OnlineAccounts",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None::<&Variant>,
            None::<&glib::VariantTy>,
            gio::DBusCallFlags::NONE,
            5_000,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string().into_boxed_str())?;
    let objects = reply.child_value(0);
    let mut accounts = Vec::new();
    for index in 0..objects.n_children() {
        let object_entry = objects.child_value(index);
        let object_path = variant_text(&object_entry.child_value(0))?;
        let interfaces = parse_goa_interfaces(&object_entry.child_value(1))?;
        let Some(account_props) = interfaces.get("org.gnome.OnlineAccounts.Account") else {
            continue;
        };
        let Some(mail_props) = interfaces.get("org.gnome.OnlineAccounts.Mail") else {
            continue;
        };
        if property_bool(account_props, "MailDisabled").unwrap_or(false) {
            continue;
        }
        if !property_bool(mail_props, "ImapSupported").unwrap_or(false) {
            continue;
        }
        let auth = if interfaces.contains_key("org.gnome.OnlineAccounts.OAuth2Based") {
            let reply = connection
                .call_sync(
                    Some("org.gnome.OnlineAccounts"),
                    object_path.as_str(),
                    "org.gnome.OnlineAccounts.OAuth2Based",
                    "GetAccessToken",
                    None::<&Variant>,
                    None::<&glib::VariantTy>,
                    gio::DBusCallFlags::NONE,
                    5_000,
                    None::<&gio::Cancellable>,
                )
                .map_err(|error| error.to_string().into_boxed_str())?;
            let token = variant_text(&reply.child_value(0))?;
            let expires_in = variant_int(&reply.child_value(1))?;
            let expires_at = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis() as i64
                + (expires_in * 1_000);
            ExternalSourceValue::variant_with_payload(
                "GoaMailOAuthToken",
                ExternalSourceValue::Record(BTreeMap::from([
                    ("accessToken".into(), ExternalSourceValue::Text(token.into())),
                    ("refreshToken".into(), ExternalSourceValue::variant("None")),
                    ("tokenType".into(), ExternalSourceValue::Text("Bearer".into())),
                    (
                        "expiresAt".into(),
                        ExternalSourceValue::variant_with_payload(
                            "Some",
                            ExternalSourceValue::Int(expires_at),
                        ),
                    ),
                ])),
            )
        } else if interfaces.contains_key("org.gnome.OnlineAccounts.PasswordBased") {
            let reply = connection
                .call_sync(
                    Some("org.gnome.OnlineAccounts"),
                    object_path.as_str(),
                    "org.gnome.OnlineAccounts.PasswordBased",
                    "GetPassword",
                    Some(&Variant::tuple_from_iter(["imap-password".to_variant()])),
                    None::<&glib::VariantTy>,
                    gio::DBusCallFlags::NONE,
                    5_000,
                    None::<&gio::Cancellable>,
                )
                .map_err(|error| error.to_string().into_boxed_str())?;
            ExternalSourceValue::variant_with_payload(
                "GoaMailPassword",
                ExternalSourceValue::Text(variant_text(&reply.child_value(0))?.into()),
            )
        } else {
            return Err(format!("GOA account {object_path} has no usable mail credential interface")
                .into_boxed_str());
        };
        let account = ExternalSourceValue::Record(BTreeMap::from([
            (
                "id".into(),
                ExternalSourceValue::Text(property_text(account_props, "Id")?.into()),
            ),
            (
                "provider".into(),
                ExternalSourceValue::Text(property_text(account_props, "ProviderName")?.into()),
            ),
            (
                "providerType".into(),
                ExternalSourceValue::Text(property_text(account_props, "ProviderType")?.into()),
            ),
            (
                "identity".into(),
                ExternalSourceValue::Text(property_text(account_props, "Identity")?.into()),
            ),
            (
                "presentationIdentity".into(),
                ExternalSourceValue::Text(
                    property_text(account_props, "PresentationIdentity")?.into(),
                ),
            ),
            (
                "state".into(),
                goa_state_external(
                    property_bool(account_props, "AttentionNeeded").unwrap_or(false),
                    property_bool(account_props, "IsLocked").unwrap_or(false),
                ),
            ),
            (
                "emailAddress".into(),
                ExternalSourceValue::Text(property_text(mail_props, "EmailAddress")?.into()),
            ),
            (
                "name".into(),
                ExternalSourceValue::Text(property_text(mail_props, "Name")?.into()),
            ),
            (
                "imapHost".into(),
                ExternalSourceValue::Text(property_text(mail_props, "ImapHost")?.into()),
            ),
            (
                "imapPort".into(),
                ExternalSourceValue::Int(goa_imap_port(mail_props)),
            ),
            (
                "imapUserName".into(),
                ExternalSourceValue::Text(property_text(mail_props, "ImapUserName")?.into()),
            ),
            (
                "imapUseSsl".into(),
                ExternalSourceValue::Bool(property_bool(mail_props, "ImapUseSsl").unwrap_or(true)),
            ),
            (
                "imapUseTls".into(),
                ExternalSourceValue::Bool(property_bool(mail_props, "ImapUseTls").unwrap_or(false)),
            ),
            (
                "smtpHost".into(),
                ExternalSourceValue::Text(property_text(mail_props, "SmtpHost")?.into()),
            ),
            (
                "smtpPort".into(),
                ExternalSourceValue::Int(goa_smtp_port(mail_props)),
            ),
            (
                "smtpUserName".into(),
                ExternalSourceValue::Text(property_text(mail_props, "SmtpUserName")?.into()),
            ),
            (
                "smtpUseSsl".into(),
                ExternalSourceValue::Bool(property_bool(mail_props, "SmtpUseSsl").unwrap_or(true)),
            ),
            (
                "smtpUseTls".into(),
                ExternalSourceValue::Bool(property_bool(mail_props, "SmtpUseTls").unwrap_or(false)),
            ),
            ("auth".into(), auth),
        ]));
        accounts.push(account);
    }
    Ok(ExternalSourceValue::List(accounts))
}

fn parse_goa_interfaces(
    interfaces: &Variant,
) -> Result<BTreeMap<String, BTreeMap<String, Variant>>, Box<str>> {
    let mut mapped = BTreeMap::new();
    for index in 0..interfaces.n_children() {
        let entry = interfaces.child_value(index);
        let interface_name = variant_text(&entry.child_value(0))?;
        let properties = entry.child_value(1);
        let mut props = BTreeMap::new();
        for prop_index in 0..properties.n_children() {
            let prop_entry = properties.child_value(prop_index);
            let prop_name = variant_text(&prop_entry.child_value(0))?;
            props.insert(prop_name, unwrap_variant(&prop_entry.child_value(1)));
        }
        mapped.insert(interface_name, props);
    }
    Ok(mapped)
}

fn unwrap_variant(value: &Variant) -> Variant {
    if value.classify() == VariantClass::Variant {
        value.as_variant().unwrap_or_else(|| value.clone())
    } else {
        value.clone()
    }
}

fn property_text(properties: &BTreeMap<String, Variant>, name: &str) -> Result<String, Box<str>> {
    let value = properties
        .get(name)
        .ok_or_else(|| format!("missing GOA property `{name}`").into_boxed_str())?;
    variant_text(value)
}

fn property_bool(properties: &BTreeMap<String, Variant>, name: &str) -> Result<bool, Box<str>> {
    let value = properties
        .get(name)
        .ok_or_else(|| format!("missing GOA property `{name}`").into_boxed_str())?;
    variant_bool(value)
}

fn variant_text(value: &Variant) -> Result<String, Box<str>> {
    value.str()
        .map(|value| value.to_owned())
        .ok_or_else(|| "expected text/object-path/signature D-Bus payload".into())
}

fn variant_bool(value: &Variant) -> Result<bool, Box<str>> {
    value
        .get::<bool>()
        .ok_or_else(|| "expected boolean D-Bus payload".into())
}

fn variant_int(value: &Variant) -> Result<i64, Box<str>> {
    if let Some(value) = value.get::<i32>() {
        return Ok(value as i64);
    }
    if let Some(value) = value.get::<i64>() {
        return Ok(value);
    }
    Err("expected integer D-Bus payload".into())
}

fn goa_state_external(attention_needed: bool, is_locked: bool) -> ExternalSourceValue {
    if is_locked {
        ExternalSourceValue::variant("AccountDisabled")
    } else if attention_needed {
        ExternalSourceValue::variant("AccountNeedsAttention")
    } else {
        ExternalSourceValue::variant("AccountActive")
    }
}

fn goa_imap_port(properties: &BTreeMap<String, Variant>) -> i64 {
    if property_bool(properties, "ImapUseSsl").unwrap_or(true) {
        993
    } else {
        143
    }
}

fn goa_smtp_port(properties: &BTreeMap<String, Variant>) -> i64 {
    if property_bool(properties, "SmtpUseSsl").unwrap_or(true) {
        465
    } else if property_bool(properties, "SmtpUseTls").unwrap_or(false) {
        587
    } else {
        25
    }
}

fn imap_connect_value(plan: &ImapConnectPlan) -> Result<RuntimeValue, Box<str>> {
    let mut snapshots = Vec::with_capacity(plan.accounts.len());
    for account in &plan.accounts {
        snapshots.push(fetch_imap_snapshot_external(account, plan.mailbox.as_ref(), plan.limit)?);
    }
    decode_ok_external(
        plan.instance,
        BuiltinSourceProvider::ImapConnect,
        &plan.result.decode,
        ExternalSourceValue::List(snapshots),
    )
}

fn imap_body_value(plan: &ImapFetchBodyPlan) -> Result<RuntimeValue, Box<str>> {
    let raw = fetch_imap_body_bytes(
        &plan.request.account,
        plan.request.mailbox.as_ref(),
        plan.request.uid,
    )?;
    let parsed = mailparse::parse_mail(&raw).map_err(|error| error.to_string().into_boxed_str())?;
    let text = extract_mail_text(&parsed, "text/plain").unwrap_or_default();
    let html = extract_mail_text(&parsed, "text/html").unwrap_or_default();
    decode_ok_external(
        plan.instance,
        BuiltinSourceProvider::ImapFetchBody,
        &plan.result.decode,
        ExternalSourceValue::Record(BTreeMap::from([
            (
                "accountId".into(),
                ExternalSourceValue::Text(plan.request.account.account_id.clone()),
            ),
            (
                "mailbox".into(),
                ExternalSourceValue::Text(plan.request.mailbox.clone()),
            ),
            ("uid".into(), ExternalSourceValue::Int(plan.request.uid as i64)),
            ("text".into(), ExternalSourceValue::Text(text.into_boxed_str())),
            ("html".into(), ExternalSourceValue::Text(html.into_boxed_str())),
            (
                "raw".into(),
                ExternalSourceValue::Text(String::from_utf8_lossy(&raw).into_owned().into_boxed_str()),
            ),
        ])),
    )
}

fn fetch_imap_snapshot_external(
    account: &ImapAccountConfig,
    mailbox: &str,
    limit: usize,
) -> Result<ExternalSourceValue, Box<str>> {
    let mut session = open_imap_session(account)?;
    session
        .select(mailbox)
        .map_err(|error| format!("failed to select {mailbox}: {error}").into_boxed_str())?;
    let mut uids = session
        .uid_search("1:*")
        .map_err(|error| format!("failed to enumerate mailbox UIDs: {error}").into_boxed_str())?
        .into_iter()
        .collect::<Vec<_>>();
    uids.sort_unstable();
    let highest_uid = uids.last().copied();
    let selected = uids
        .into_iter()
        .rev()
        .take(limit.max(1))
        .collect::<Vec<_>>();
    let sequence = selected
        .iter()
        .rev()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let messages = if sequence.is_empty() {
        Vec::new()
    } else {
        session
            .uid_fetch(sequence, "UID FLAGS BODY.PEEK[HEADER]")
            .map_err(|error| format!("failed to fetch mailbox headers: {error}").into_boxed_str())?
            .iter()
            .map(|message| {
                let uid = message
                    .uid
                    .ok_or_else(|| "imap fetch did not include UID".to_owned())?;
                let header = message.header().or_else(|| message.body()).unwrap_or_default();
                let header_text = String::from_utf8_lossy(header).into_owned();
                let parsed = mailparse::parse_headers(header)
                    .map_err(|error| error.to_string())?
                    .0;
                Ok::<ExternalSourceValue, String>(ExternalSourceValue::Record(BTreeMap::from([
                    ("uid".into(), ExternalSourceValue::Int(uid as i64)),
                    (
                        "subject".into(),
                        ExternalSourceValue::Text(
                            parsed
                                .get_first_value("Subject")
                                .unwrap_or_default()
                                .into_boxed_str(),
                        ),
                    ),
                    (
                        "from".into(),
                        ExternalSourceValue::Text(
                            parsed
                                .get_first_value("From")
                                .unwrap_or_default()
                                .into_boxed_str(),
                        ),
                    ),
                    (
                        "date".into(),
                        ExternalSourceValue::Text(
                            parsed
                                .get_first_value("Date")
                                .unwrap_or_default()
                                .into_boxed_str(),
                        ),
                    ),
                    (
                        "messageId".into(),
                        ExternalSourceValue::Text(
                            parsed
                                .get_first_value("Message-Id")
                                .unwrap_or_default()
                                .into_boxed_str(),
                        ),
                    ),
                    (
                        "flags".into(),
                        ExternalSourceValue::List(
                            message
                                .flags()
                                .iter()
                                .filter_map(imap_flag_external)
                                .collect(),
                        ),
                    ),
                    ("preview".into(), ExternalSourceValue::Text("".into())),
                    (
                        "rawHeader".into(),
                        ExternalSourceValue::Text(header_text.into_boxed_str()),
                    ),
                ])))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.into_boxed_str())?
    };
    let _ = session.logout();
    Ok(ExternalSourceValue::Record(BTreeMap::from([
        (
            "accountId".into(),
            ExternalSourceValue::Text(account.account_id.clone()),
        ),
        ("mailbox".into(), ExternalSourceValue::Text(mailbox.into())),
        (
            "highestUid".into(),
            highest_uid
                .map(|uid| ExternalSourceValue::variant_with_payload("Some", ExternalSourceValue::Int(uid as i64)))
                .unwrap_or_else(|| ExternalSourceValue::variant("None")),
        ),
        ("messages".into(), ExternalSourceValue::List(messages)),
    ])))
}

fn fetch_imap_body_bytes(
    account: &ImapAccountConfig,
    mailbox: &str,
    uid: u32,
) -> Result<Vec<u8>, Box<str>> {
    let mut session = open_imap_session(account)?;
    session
        .select(mailbox)
        .map_err(|error| format!("failed to select {mailbox}: {error}").into_boxed_str())?;
    let fetches = session
        .uid_fetch(uid.to_string(), "BODY.PEEK[]")
        .map_err(|error| format!("failed to fetch message body: {error}").into_boxed_str())?;
    let body = fetches
        .iter()
        .next()
        .and_then(|message| message.body())
        .map(|body| body.to_vec())
        .ok_or_else(|| format!("message UID {uid} has no fetchable body").into_boxed_str())?;
    let _ = session.logout();
    Ok(body)
}

fn imap_highest_uid(account: &ImapAccountConfig, mailbox: &str) -> Result<Option<u32>, Box<str>> {
    let mut session = open_imap_session(account)?;
    session
        .select(mailbox)
        .map_err(|error| format!("failed to select {mailbox}: {error}").into_boxed_str())?;
    let mut uids = session
        .uid_search("1:*")
        .map_err(|error| format!("failed to enumerate mailbox UIDs: {error}").into_boxed_str())?
        .into_iter()
        .collect::<Vec<_>>();
    uids.sort_unstable();
    let result = uids.last().copied();
    let _ = session.logout();
    Ok(result)
}

trait ImapIo: Read + Write + Send {}
impl<T: Read + Write + Send> ImapIo for T {}

struct Xoauth2Authenticator {
    payload: Box<str>,
}

impl imap::Authenticator for Xoauth2Authenticator {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> Self::Response {
        BASE64_STANDARD.encode(self.payload.as_bytes())
    }
}

fn open_imap_session(
    account: &ImapAccountConfig,
) -> Result<imap::Session<Box<dyn ImapIo>>, Box<str>> {
    if account.use_tls && !account.use_ssl {
        return Err(
            "STARTTLS-backed GOA IMAP accounts are not executed by this runtime slice yet".into(),
        );
    }
    let tcp = TcpStream::connect((account.host.as_ref(), account.port))
        .map_err(|error| format!("failed to connect to {}:{}: {error}", account.host, account.port).into_boxed_str())?;
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let client = if account.use_ssl {
        let tls = TlsConnector::builder()
            .build()
            .map_err(|error| error.to_string().into_boxed_str())?;
        let stream = tls
            .connect(account.host.as_ref(), tcp)
            .map_err(|error| error.to_string().into_boxed_str())?;
        imap::Client::new(Box::new(stream) as Box<dyn ImapIo>)
    } else {
        imap::Client::new(Box::new(tcp) as Box<dyn ImapIo>)
    };
    match &account.auth {
        ImapAuthConfig::Password(password) => client
            .login(account.user.as_ref(), password.as_ref())
            .map_err(|(error, _)| format!("IMAP login failed: {error}").into_boxed_str()),
        ImapAuthConfig::OAuthToken(token) => {
            let payload = format!(
                "user={}\x01auth=Bearer {}\x01\x01",
                account.user.as_ref(),
                token.as_ref()
            );
            client
                .authenticate("XOAUTH2", &mut Xoauth2Authenticator { payload: payload.into() })
                .map_err(|(error, _)| {
                    format!("IMAP XOAUTH2 authentication failed: {error}").into_boxed_str()
                })
        }
    }
}

fn imap_flag_external(flag: &imap::types::Flag<'_>) -> Option<ExternalSourceValue> {
    match flag {
        imap::types::Flag::Seen => Some(ExternalSourceValue::variant("Seen")),
        imap::types::Flag::Answered => Some(ExternalSourceValue::variant("Answered")),
        imap::types::Flag::Flagged => Some(ExternalSourceValue::variant("Flagged")),
        imap::types::Flag::Draft => Some(ExternalSourceValue::variant("Draft")),
        _ => None,
    }
}

fn extract_mail_text(parsed: &mailparse::ParsedMail<'_>, mime: &str) -> Option<String> {
    if parsed.subparts.is_empty() {
        return (parsed.ctype.mimetype.eq_ignore_ascii_case(mime))
            .then(|| parsed.get_body().ok())
            .flatten();
    }
    parsed
        .subparts
        .iter()
        .find_map(|part| extract_mail_text(part, mime))
}

fn parse_imap_accounts_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Vec<ImapAccountConfig>, SourceProviderExecutionError> {
    let RuntimeValue::List(values) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "List GoaMailAccount".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    values
        .iter()
        .map(|value| parse_imap_account_value(instance, provider, index, strip_signal(value)))
        .collect()
}

fn parse_imap_account_value(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &RuntimeValue,
) -> Result<ImapAccountConfig, SourceProviderExecutionError> {
    let RuntimeValue::Record(fields) = value else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "GoaMailAccount record".into(),
            value: value.clone(),
        });
    };
    Ok(ImapAccountConfig {
        account_id: record_text_field(instance, provider, index, fields, "id")?,
        host: record_text_field(instance, provider, index, fields, "imapHost")?,
        port: record_int_field(instance, provider, index, fields, "imapPort")? as u16,
        user: record_text_field(instance, provider, index, fields, "imapUserName")?,
        use_ssl: record_bool_field(instance, provider, index, fields, "imapUseSsl")?,
        use_tls: record_bool_field(instance, provider, index, fields, "imapUseTls")?,
        auth: parse_goa_mail_auth(instance, provider, index, fields)?,
    })
}

fn parse_goa_mail_auth(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    fields: &[aivi_backend::RuntimeRecordField],
) -> Result<ImapAuthConfig, SourceProviderExecutionError> {
    let value = record_field(fields, "auth").ok_or_else(|| SourceProviderExecutionError::InvalidArgument {
        instance,
        provider,
        index,
        expected: "GoaMailAccount.auth".into(),
        value: RuntimeValue::Record(fields.to_vec()),
    })?;
    let RuntimeValue::Sum(sum) = strip_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "GoaMailAuth".into(),
            value: strip_signal(value).clone(),
        });
    };
    match sum.variant_name.as_ref() {
        "GoaMailPassword" => {
            let Some(RuntimeValue::Text(value)) = sum.fields.first() else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "GoaMailPassword Text".into(),
                    value: strip_signal(value).clone(),
                });
            };
            Ok(ImapAuthConfig::Password(value.clone()))
        }
        "GoaMailOAuthToken" => {
            let Some(RuntimeValue::Record(fields)) = sum.fields.first() else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "GoaMailOAuthToken OAuthToken".into(),
                    value: strip_signal(value).clone(),
                });
            };
            Ok(ImapAuthConfig::OAuthToken(record_text_field(
                instance,
                provider,
                index,
                fields,
                "accessToken",
            )?))
        }
        _ => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "GoaMailPassword or GoaMailOAuthToken".into(),
            value: strip_signal(value).clone(),
        }),
    }
}

fn parse_imap_body_request(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<ImapBodyRequest, SourceProviderExecutionError> {
    let RuntimeValue::Record(fields) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "ImapBodyRequest record".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    Ok(ImapBodyRequest {
        account: parse_imap_account_value(
            instance,
            provider,
            index,
            &RuntimeValue::Record(fields.to_vec()),
        )?,
        mailbox: record_text_field(instance, provider, index, fields, "mailbox")?,
        uid: record_int_field(instance, provider, index, fields, "uid")? as u32,
    })
}

fn record_field<'a>(
    fields: &'a [aivi_backend::RuntimeRecordField],
    name: &str,
) -> Option<&'a RuntimeValue> {
    fields
        .iter()
        .find(|field| field.label.as_ref() == name)
        .map(|field| &field.value)
}

fn record_text_field(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    fields: &[aivi_backend::RuntimeRecordField],
    name: &str,
) -> Result<Box<str>, SourceProviderExecutionError> {
    let Some(value) = record_field(fields, name) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Text`").into_boxed_str(),
            value: RuntimeValue::Record(fields.to_vec()),
        });
    };
    let RuntimeValue::Text(value) = strip_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Text`").into_boxed_str(),
            value: strip_signal(value).clone(),
        });
    };
    Ok(value.clone())
}

fn record_bool_field(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    fields: &[aivi_backend::RuntimeRecordField],
    name: &str,
) -> Result<bool, SourceProviderExecutionError> {
    let Some(value) = record_field(fields, name) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Bool`").into_boxed_str(),
            value: RuntimeValue::Record(fields.to_vec()),
        });
    };
    let RuntimeValue::Bool(value) = strip_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Bool`").into_boxed_str(),
            value: strip_signal(value).clone(),
        });
    };
    Ok(*value)
}

fn record_int_field(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    fields: &[aivi_backend::RuntimeRecordField],
    name: &str,
) -> Result<i64, SourceProviderExecutionError> {
    let Some(value) = record_field(fields, name) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Int`").into_boxed_str(),
            value: RuntimeValue::Record(fields.to_vec()),
        });
    };
    let RuntimeValue::Int(value) = strip_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: format!("record field `{name}: Int`").into_boxed_str(),
            value: strip_signal(value).clone(),
        });
    };
    Ok(*value)
}

fn decode_ok_external(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    decode: &hir::SourceDecodeProgram,
    payload: ExternalSourceValue,
) -> Result<RuntimeValue, Box<str>> {
    decode_external(
        decode,
        &ExternalSourceValue::variant_with_payload("Ok", payload),
    )
    .map_err(|error| {
        format!(
            "provider {} success payload does not match current source output shape for instance {}: {error}",
            provider.key(),
            instance.as_raw()
        )
        .into_boxed_str()
    })
}

fn decode_err_external(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    decode: &hir::SourceDecodeProgram,
    payload: ExternalSourceValue,
) -> Result<RuntimeValue, Box<str>> {
    decode_external(
        decode,
        &ExternalSourceValue::variant_with_payload("Err", payload),
    )
    .map_err(|error| {
        format!(
            "provider {} error payload does not match current source output shape for instance {}: {error}",
            provider.key(),
            instance.as_raw()
        )
        .into_boxed_str()
    })
}

fn imap_error_external(detail: &str) -> ExternalSourceValue {
    if detail.contains("AUTH") || detail.contains("Invalid credentials") {
        ExternalSourceValue::variant("ImapAuthFailed")
    } else {
        ExternalSourceValue::variant_with_payload(
            "ImapConnectionFailed",
            ExternalSourceValue::Text(detail.into()),
        )
    }
}
