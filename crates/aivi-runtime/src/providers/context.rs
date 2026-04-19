/// Scheduler-owned mailbox routing table.
///
/// Each mailbox stores only the subscribers that are currently live. The inner
/// map key is the stable subscriber id handed back to the source manager so
/// suspend/reconfigure teardown can remove the exact mailbox worker it started.
#[derive(Default)]
struct MailboxHub {
    next_id: u64,
    subscribers: BTreeMap<Box<str>, BTreeMap<u64, SyncSender<Box<str>>>>,
}

impl MailboxHub {
    fn subscribe(&mut self, mailbox: &str, buffer: usize) -> (u64, mpsc::Receiver<Box<str>>) {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("mailbox subscriber ids should not overflow");
        let (sender, receiver) = mpsc::sync_channel(buffer.max(1));
        let replaced = self
            .subscribers
            .entry(mailbox.into())
            .or_default()
            .insert(id, sender);
        debug_assert!(
            replaced.is_none(),
            "mailbox subscriber ids must stay unique within a hub"
        );
        (id, receiver)
    }

    fn unsubscribe(&mut self, mailbox: &str, id: u64) {
        let should_remove = {
            let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
                return;
            };
            subscribers.remove(&id);
            subscribers.is_empty()
        };
        if should_remove {
            self.subscribers.remove(mailbox);
        }
    }

    fn publish(&mut self, mailbox: &str, message: &str) -> Result<(), MailboxPublishError> {
        let (full, should_remove) = {
            let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
                return Ok(());
            };
            let mut full = false;
            let mut disconnected = Vec::new();
            for (&id, sender) in subscribers.iter() {
                match sender.try_send(message.into()) {
                    Ok(()) => {}
                    Err(TrySendError::Disconnected(_)) => disconnected.push(id),
                    Err(TrySendError::Full(_)) => {
                        full = true;
                    }
                }
            }
            for id in disconnected {
                subscribers.remove(&id);
            }
            (full, subscribers.is_empty())
        };
        if should_remove {
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
pub struct SourceProviderContext {
    args: Arc<[String]>,
    cwd: Arc<PathBuf>,
    app_dir: Arc<PathBuf>,
    env: Arc<BTreeMap<String, String>>,
    stdin_override: Option<Result<Box<str>, Box<str>>>,
    stdin_text: Arc<OnceLock<Result<Box<str>, Box<str>>>>,
    custom_capability_command_executor: Option<Arc<dyn CustomCapabilityCommandExecutor>>,
}

impl Default for SourceProviderContext {
    fn default() -> Self {
        Self::current()
    }
}

impl SourceProviderContext {
    pub fn current() -> Self {
        let args = env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let env = env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Self::new(args, cwd, env)
    }

    pub fn new(args: Vec<String>, cwd: PathBuf, env: BTreeMap<String, String>) -> Self {
        Self {
            args: Arc::from(args.into_boxed_slice()),
            app_dir: Arc::new(cwd.clone()),
            cwd: Arc::new(cwd),
            env: Arc::new(env),
            stdin_override: None,
            stdin_text: Arc::new(OnceLock::new()),
            custom_capability_command_executor: None,
        }
    }

    pub fn with_app_dir(mut self, app_dir: PathBuf) -> Self {
        self.app_dir = Arc::new(app_dir);
        self
    }

    pub fn with_entry_path(self, entry_path: &Path) -> Self {
        let app_dir = entry_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        self.with_app_dir(app_dir)
    }

    pub fn app_dir(&self) -> &Path {
        self.app_dir.as_ref()
    }

    pub fn with_stdin_text(mut self, stdin: impl Into<String>) -> Self {
        self.stdin_override = Some(Ok(stdin.into().into_boxed_str()));
        self
    }

    pub fn with_custom_capability_command_executor(
        mut self,
        executor: Arc<dyn CustomCapabilityCommandExecutor>,
    ) -> Self {
        self.custom_capability_command_executor = Some(executor);
        self
    }

    pub(crate) fn custom_capability_command_executor(
        &self,
    ) -> Option<&Arc<dyn CustomCapabilityCommandExecutor>> {
        self.custom_capability_command_executor.as_ref()
    }

    fn args_runtime_value(&self) -> RuntimeValue {
        RuntimeValue::List(
            self.args
                .iter()
                .cloned()
                .map(|arg| RuntimeValue::Text(arg.into_boxed_str()))
                .collect(),
        )
    }

    fn cwd_runtime_value(&self) -> RuntimeValue {
        RuntimeValue::Text(self.cwd.to_string_lossy().into_owned().into_boxed_str())
    }

    fn app_dir_runtime_value(&self) -> RuntimeValue {
        RuntimeValue::Text(self.app_dir.to_string_lossy().into_owned().into_boxed_str())
    }

    fn env_runtime_value(&self, key: &str) -> RuntimeValue {
        match self.env.get(key) {
            Some(value) => RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(
                value.clone().into_boxed_str(),
            ))),
            None => RuntimeValue::OptionNone,
        }
    }

    fn stdin_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(value) = &self.stdin_override {
            return value.clone();
        }
        self.stdin_text
            .get_or_init(|| {
                let mut input = String::new();
                std::io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|error| format!("failed to read stdin: {error}").into_boxed_str())?;
                Ok(input.into_boxed_str())
            })
            .clone()
    }

    fn home_dir_text(&self) -> Result<Box<str>, Box<str>> {
        self.env
            .get("HOME")
            .cloned()
            .map(String::into_boxed_str)
            .ok_or_else(|| "HOME is not set".into())
    }

    fn config_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_CONFIG_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".config")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn data_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_DATA_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".local")
            .join("share")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn cache_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_CACHE_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".cache")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn temp_dir_text(&self) -> Box<str> {
        env::temp_dir()
            .to_string_lossy()
            .into_owned()
            .into_boxed_str()
    }

    fn normalize_sqlite_database_text(&self, database: &str) -> Box<str> {
        if database == ":memory:" || database.starts_with("file:") {
            return database.into();
        }
        let path = PathBuf::from(database);
        let resolved = if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        };
        resolved.to_string_lossy().into_owned().into_boxed_str()
    }
}
