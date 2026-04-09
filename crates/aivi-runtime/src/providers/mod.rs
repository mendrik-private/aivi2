use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fmt, fs,
    io::{BufRead, BufReader, Read},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use aivi_backend::{DetachedRuntimeValue, RuntimeCallable, RuntimeValue};
use aivi_hir as hir;
use aivi_typing::BuiltinSourceProvider;
use gio::{
    BusNameOwnerFlags, BusType, DBusConnection, DBusConnectionFlags, DBusMessageType,
    DBusSendMessageFlags, DBusSignalFlags,
};
use glib::{ControlFlow, MainContext, MainLoop, Variant, VariantClass};
use url::Url;

use crate::startup::DetachedRuntimePublicationPort;
use crate::{
    CancellationObserver, EvaluatedSourceConfig, LinkedSourceLifecycleAction,
    RuntimeSourceProvider, SourceInstanceId, SourceLifecycleActionKind,
    source_decode::{
        ExternalSourceValue, SourceDecodeError, SourceDecodeErrorWithPath, decode_external,
        encode_runtime_json, parse_json_text, validate_supported_program,
    },
    task_executor::{
        CustomCapabilityCommandExecutor, execute_runtime_value_with_context_with_stdio,
    },
};

include!("context.rs");
include!("manager.rs");
include!("plans.rs");
include!("runtime.rs");

#[cfg(test)]
mod tests;
