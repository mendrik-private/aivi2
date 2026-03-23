use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use aivi_backend::{
    EvaluationError, ItemId as BackendItemId, ItemKind as BackendItemKind, KernelEvaluator,
    KernelId, Program as BackendProgram, RuntimeValue, SourceId as BackendSourceId,
};
use aivi_core as core;
use aivi_hir as hir;

use crate::{
    InputHandle, RuntimeSourceProvider, SourceInstanceId, SourceLifecycleActionKind,
    SourcePublicationPort, TaskSourceRuntime, TaskSourceRuntimeError, TickOutcome,
    TryDerivedNodeEvaluator,
    graph::{DerivedHandle, OwnerHandle, SignalHandle},
    hir_adapter::{HirRuntimeAssembly, HirRuntimeInstantiationError},
    scheduler::DependencyValues,
};

pub fn link_backend_runtime<'a>(
    assembly: HirRuntimeAssembly,
    core: &core::Module,
    backend: &'a BackendProgram,
) -> Result<BackendLinkedRuntime<'a>, BackendRuntimeLinkErrors> {
    let runtime = assembly
        .instantiate_runtime::<RuntimeValue>()
        .map_err(|error| {
            BackendRuntimeLinkErrors::new(vec![BackendRuntimeLinkError::InstantiateRuntime {
                error,
            }])
        })?;
    let mut builder = LinkBuilder::new(&assembly, core, backend);
    let linked = builder.build()?;
    Ok(BackendLinkedRuntime {
        assembly,
        runtime,
        backend,
        signal_items_by_handle: linked.signal_items_by_handle,
        runtime_signal_by_item: linked.runtime_signal_by_item,
        derived_signals: linked.derived_signals,
        source_bindings: linked.source_bindings,
    })
}

pub struct BackendLinkedRuntime<'a> {
    assembly: HirRuntimeAssembly,
    runtime: TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram>,
    backend: &'a BackendProgram,
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
}

impl<'a> BackendLinkedRuntime<'a> {
    pub fn assembly(&self) -> &HirRuntimeAssembly {
        &self.assembly
    }

    pub fn backend(&self) -> &'a BackendProgram {
        self.backend
    }

    pub fn runtime(&self) -> &TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram> {
        &self.runtime
    }

    pub fn runtime_mut(
        &mut self,
    ) -> &mut TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram> {
        &mut self.runtime
    }

    pub fn derived_signal(&self, signal: DerivedHandle) -> Option<&LinkedDerivedSignal> {
        self.derived_signals.get(&signal)
    }

    pub fn source_binding(&self, instance: SourceInstanceId) -> Option<&LinkedSourceBinding> {
        self.source_bindings.get(&instance)
    }

    pub fn source_by_owner(&self, owner: hir::ItemId) -> Option<&LinkedSourceBinding> {
        self.source_bindings
            .values()
            .find(|binding| binding.owner == owner)
    }

    pub fn tick(&mut self) -> Result<TickOutcome, BackendRuntimeError> {
        let committed = self.committed_signal_snapshots()?;
        let mut evaluator = LinkedDerivedEvaluator {
            backend: self.backend,
            derived_signals: &self.derived_signals,
            committed_signals: &committed,
        };
        self.runtime.try_tick(&mut evaluator)
    }

    pub fn tick_with_source_lifecycle(
        &mut self,
    ) -> Result<LinkedSourceTickOutcome, BackendRuntimeError> {
        let scheduler = self.tick()?;
        let mut committed = vec![false; self.runtime.graph().signal_count()];
        for &signal in scheduler.committed() {
            committed[signal.index()] = true;
        }

        let instances = self.source_bindings.keys().copied().collect::<Vec<_>>();
        let mut actions = Vec::new();
        for instance in instances {
            let binding = self
                .source_bindings
                .get(&instance)
                .expect("linked source binding should exist");
            if !self.runtime.is_owner_active(binding.owner_handle)? {
                continue;
            }

            let spec = self
                .runtime
                .source_spec(instance)
                .expect("linked runtime should preserve registered source specs");
            let should_be_active = match spec.active_when {
                Some(signal) => {
                    let value = self.runtime.current_value(signal)?;
                    active_when_value(instance, value)?
                }
                None => true,
            };
            if !should_be_active {
                if self.runtime.is_source_active(instance) {
                    self.runtime.suspend_source(instance)?;
                    actions.push(LinkedSourceLifecycleAction::Suspend { instance });
                }
                continue;
            }

            if !self.runtime.is_source_active(instance) {
                let config = self.evaluate_source_config(instance)?;
                let port = self.runtime.activate_source(instance)?;
                actions.push(LinkedSourceLifecycleAction::Activate {
                    instance,
                    port,
                    config,
                });
                continue;
            }

            let dependency_changed = spec
                .reconfiguration_dependencies
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            let trigger_changed = spec
                .explicit_triggers
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            if dependency_changed || trigger_changed {
                let config = self.evaluate_source_config(instance)?;
                let port = self.runtime.reconfigure_source(instance)?;
                actions.push(LinkedSourceLifecycleAction::Reconfigure {
                    instance,
                    port,
                    config,
                });
            }
        }

        Ok(LinkedSourceTickOutcome {
            scheduler,
            source_actions: actions.into_boxed_slice(),
        })
    }

    pub fn evaluate_source_config(
        &self,
        instance: SourceInstanceId,
    ) -> Result<EvaluatedSourceConfig, BackendRuntimeError> {
        let binding = self
            .source_bindings
            .get(&instance)
            .ok_or(BackendRuntimeError::UnknownSourceInstance { instance })?;
        let snapshots = self.committed_signal_snapshots()?;
        let mut evaluator = KernelEvaluator::new(self.backend);
        let mut arguments = Vec::with_capacity(binding.arguments.len());
        for (index, argument) in binding.arguments.iter().enumerate() {
            let globals = self.required_signal_globals(
                instance,
                argument.kernel,
                &argument.required_signals,
                &snapshots,
            )?;
            let value = evaluator
                .evaluate_kernel(argument.kernel, None, &[], &globals)
                .map_err(|error| BackendRuntimeError::EvaluateSourceArgument {
                    instance,
                    index,
                    error,
                })?;
            arguments.push(value);
        }
        let mut options = Vec::with_capacity(binding.options.len());
        for option in &binding.options {
            let globals = self.required_signal_globals(
                instance,
                option.kernel,
                &option.required_signals,
                &snapshots,
            )?;
            let value = evaluator
                .evaluate_kernel(option.kernel, None, &[], &globals)
                .map_err(|error| BackendRuntimeError::EvaluateSourceOption {
                    instance,
                    option_name: option.option_name.clone(),
                    error,
                })?;
            options.push(EvaluatedSourceOption {
                option_name: option.option_name.clone(),
                value,
            });
        }

        Ok(EvaluatedSourceConfig {
            owner: binding.owner,
            instance,
            source: binding.backend_source,
            provider: self
                .runtime
                .source_spec(instance)
                .expect("linked runtime should preserve registered source specs")
                .provider
                .clone(),
            arguments: arguments.into_boxed_slice(),
            options: options.into_boxed_slice(),
        })
    }

    fn committed_signal_snapshots(
        &self,
    ) -> Result<BTreeMap<BackendItemId, RuntimeValue>, BackendRuntimeError> {
        let mut snapshots = BTreeMap::new();
        for (&signal, &item) in &self.signal_items_by_handle {
            if let Some(value) = self.runtime.current_value(signal)? {
                snapshots.insert(item, RuntimeValue::Signal(Box::new(value.clone())));
            }
        }
        Ok(snapshots)
    }

    fn required_signal_globals(
        &self,
        instance: SourceInstanceId,
        kernel: KernelId,
        required: &[BackendItemId],
        snapshots: &BTreeMap<BackendItemId, RuntimeValue>,
    ) -> Result<BTreeMap<BackendItemId, RuntimeValue>, BackendRuntimeError> {
        let mut globals = BTreeMap::new();
        for item in required {
            let signal = self.runtime_signal_by_item.get(item).copied().ok_or(
                BackendRuntimeError::MissingSignalItemMapping {
                    instance,
                    kernel,
                    item: *item,
                },
            )?;
            let value = snapshots.get(item).cloned().ok_or(
                BackendRuntimeError::MissingCommittedSignalSnapshot {
                    instance,
                    kernel,
                    signal,
                },
            )?;
            globals.insert(*item, value);
        }
        Ok(globals)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedDerivedSignal {
    pub item: hir::ItemId,
    pub signal: DerivedHandle,
    pub backend_item: BackendItemId,
    pub dependency_items: Box<[BackendItemId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceBinding {
    pub owner: hir::ItemId,
    pub owner_handle: OwnerHandle,
    pub signal: SignalHandle,
    pub input: InputHandle,
    pub instance: SourceInstanceId,
    pub backend_owner: BackendItemId,
    pub backend_source: BackendSourceId,
    pub arguments: Box<[LinkedSourceArgument]>,
    pub options: Box<[LinkedSourceOption]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceArgument {
    pub kernel: KernelId,
    pub required_signals: Box<[BackendItemId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceOption {
    pub option_name: Box<str>,
    pub kernel: KernelId,
    pub required_signals: Box<[BackendItemId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedSourceConfig {
    pub owner: hir::ItemId,
    pub instance: SourceInstanceId,
    pub source: BackendSourceId,
    pub provider: RuntimeSourceProvider,
    pub arguments: Box<[RuntimeValue]>,
    pub options: Box<[EvaluatedSourceOption]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedSourceOption {
    pub option_name: Box<str>,
    pub value: RuntimeValue,
}

pub struct LinkedSourceTickOutcome {
    scheduler: TickOutcome,
    source_actions: Box<[LinkedSourceLifecycleAction]>,
}

impl LinkedSourceTickOutcome {
    pub fn scheduler(&self) -> &TickOutcome {
        &self.scheduler
    }

    pub fn source_actions(&self) -> &[LinkedSourceLifecycleAction] {
        &self.source_actions
    }
}

#[derive(Clone)]
pub enum LinkedSourceLifecycleAction {
    Activate {
        instance: SourceInstanceId,
        port: SourcePublicationPort<RuntimeValue>,
        config: EvaluatedSourceConfig,
    },
    Reconfigure {
        instance: SourceInstanceId,
        port: SourcePublicationPort<RuntimeValue>,
        config: EvaluatedSourceConfig,
    },
    Suspend {
        instance: SourceInstanceId,
    },
}

impl LinkedSourceLifecycleAction {
    pub const fn kind(&self) -> SourceLifecycleActionKind {
        match self {
            Self::Activate { .. } => SourceLifecycleActionKind::Activate,
            Self::Reconfigure { .. } => SourceLifecycleActionKind::Reconfigure,
            Self::Suspend { .. } => SourceLifecycleActionKind::Suspend,
        }
    }

    pub const fn instance(&self) -> SourceInstanceId {
        match self {
            Self::Activate { instance, .. }
            | Self::Reconfigure { instance, .. }
            | Self::Suspend { instance } => *instance,
        }
    }

    pub fn config(&self) -> Option<&EvaluatedSourceConfig> {
        match self {
            Self::Activate { config, .. } | Self::Reconfigure { config, .. } => Some(config),
            Self::Suspend { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendRuntimeLinkError {
    InstantiateRuntime {
        error: HirRuntimeInstantiationError,
    },
    MissingCoreItemOrigin {
        backend_item: BackendItemId,
        core_item: core::ItemId,
    },
    DuplicateBackendOrigin {
        item: hir::ItemId,
        first: BackendItemId,
        second: BackendItemId,
    },
    MissingBackendItem {
        item: hir::ItemId,
    },
    BackendItemNotSignal {
        item: hir::ItemId,
        backend_item: BackendItemId,
    },
    MissingRuntimeOwner {
        owner: hir::ItemId,
    },
    MissingBackendSource {
        owner: hir::ItemId,
        backend_item: BackendItemId,
    },
    SourceInstanceMismatch {
        owner: hir::ItemId,
        runtime: SourceInstanceId,
        backend: aivi_backend::SourceInstanceId,
    },
    SourceBackedBodySignalNotYetLinked {
        item: hir::ItemId,
    },
    SignalPipelinesNotYetLinked {
        item: hir::ItemId,
        count: usize,
    },
    MissingSignalBody {
        item: hir::ItemId,
        backend_item: BackendItemId,
    },
    UnsupportedInlinePipeKernel {
        owner: hir::ItemId,
        kernel: KernelId,
    },
    MissingItemBodyForGlobal {
        owner: hir::ItemId,
        item: BackendItemId,
    },
    MissingRuntimeSignalDependency {
        owner: hir::ItemId,
        dependency: BackendItemId,
    },
    SignalRequirementMismatch {
        item: hir::ItemId,
        declared: Box<[BackendItemId]>,
        required: Box<[BackendItemId]>,
    },
    SignalDependencyMismatch {
        item: hir::ItemId,
        runtime: Box<[SignalHandle]>,
        backend: Box<[SignalHandle]>,
    },
}

impl fmt::Display for BackendRuntimeLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstantiateRuntime { error } => {
                write!(f, "runtime instantiation failed: {error}")
            }
            Self::MissingCoreItemOrigin {
                backend_item,
                core_item,
            } => write!(
                f,
                "backend item item{backend_item} points at missing core item {core_item}"
            ),
            Self::DuplicateBackendOrigin {
                item,
                first,
                second,
            } => write!(
                f,
                "HIR item {item} lowered to multiple backend items: item{first} and item{second}"
            ),
            Self::MissingBackendItem { item } => {
                write!(f, "HIR runtime item {item} has no linked backend item")
            }
            Self::BackendItemNotSignal { item, backend_item } => write!(
                f,
                "HIR signal {item} lowered to non-signal backend item item{backend_item}"
            ),
            Self::MissingRuntimeOwner { owner } => {
                write!(
                    f,
                    "runtime assembly is missing an owner binding for item {owner}"
                )
            }
            Self::MissingBackendSource {
                owner,
                backend_item,
            } => write!(
                f,
                "runtime source owner {owner} has no linked backend source on item{backend_item}"
            ),
            Self::SourceInstanceMismatch {
                owner,
                runtime,
                backend,
            } => write!(
                f,
                "runtime source instance {} for owner {owner} does not match backend source {}",
                runtime.as_raw(),
                backend.as_raw()
            ),
            Self::SourceBackedBodySignalNotYetLinked { item } => write!(
                f,
                "source-backed body signal {item} still crosses the explicit publication-to-body gap"
            ),
            Self::SignalPipelinesNotYetLinked { item, count } => write!(
                f,
                "signal {item} still has {count} backend pipeline handoff(s) that startup does not execute yet"
            ),
            Self::MissingSignalBody { item, backend_item } => write!(
                f,
                "linked derived signal {item} has no backend body kernel on item{backend_item}"
            ),
            Self::UnsupportedInlinePipeKernel { owner, kernel } => write!(
                f,
                "owner {owner} still depends on inline-pipe kernel{kernel}, which startup cannot evaluate yet"
            ),
            Self::MissingItemBodyForGlobal { owner, item } => write!(
                f,
                "owner {owner} references non-signal global item{item} without a backend body kernel"
            ),
            Self::MissingRuntimeSignalDependency { owner, dependency } => write!(
                f,
                "owner {owner} depends on backend signal item{dependency} with no runtime signal binding"
            ),
            Self::SignalRequirementMismatch {
                item,
                declared,
                required,
            } => write!(
                f,
                "signal {item} declares backend dependencies {:?}, but its reachable body requires {:?}",
                declared, required
            ),
            Self::SignalDependencyMismatch {
                item,
                runtime,
                backend,
            } => write!(
                f,
                "signal {item} runtime dependencies {:?} do not match backend dependencies {:?}",
                runtime, backend
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendRuntimeLinkErrors {
    errors: Box<[BackendRuntimeLinkError]>,
}

impl BackendRuntimeLinkErrors {
    pub fn new(errors: Vec<BackendRuntimeLinkError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[BackendRuntimeLinkError] {
        &self.errors
    }
}

impl fmt::Display for BackendRuntimeLinkErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BackendRuntimeLinkErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendRuntimeError {
    Runtime(TaskSourceRuntimeError),
    UnknownDerivedSignal {
        signal: DerivedHandle,
    },
    DerivedDependencyArityMismatch {
        signal: DerivedHandle,
        expected: usize,
        found: usize,
    },
    UnknownSourceInstance {
        instance: SourceInstanceId,
    },
    MissingCommittedSignalSnapshot {
        instance: SourceInstanceId,
        kernel: KernelId,
        signal: SignalHandle,
    },
    MissingSignalItemMapping {
        instance: SourceInstanceId,
        kernel: KernelId,
        item: BackendItemId,
    },
    EvaluateDerivedSignal {
        signal: DerivedHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    EvaluateSourceArgument {
        instance: SourceInstanceId,
        index: usize,
        error: EvaluationError,
    },
    EvaluateSourceOption {
        instance: SourceInstanceId,
        option_name: Box<str>,
        error: EvaluationError,
    },
    InvalidActiveWhenValue {
        instance: SourceInstanceId,
        value: RuntimeValue,
    },
}

impl From<TaskSourceRuntimeError> for BackendRuntimeError {
    fn from(value: TaskSourceRuntimeError) -> Self {
        Self::Runtime(value)
    }
}

impl fmt::Display for BackendRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(f, "runtime access failed: {error:?}"),
            Self::UnknownDerivedSignal { signal } => {
                write!(
                    f,
                    "startup linker does not know derived signal {:?}",
                    signal
                )
            }
            Self::DerivedDependencyArityMismatch {
                signal,
                expected,
                found,
            } => write!(
                f,
                "derived signal {:?} expected {expected} runtime dependencies, found {found}",
                signal
            ),
            Self::UnknownSourceInstance { instance } => {
                write!(
                    f,
                    "startup linker does not know source instance {}",
                    instance.as_raw()
                )
            }
            Self::MissingCommittedSignalSnapshot {
                instance,
                kernel,
                signal,
            } => write!(
                f,
                "source instance {} requires committed snapshot for signal {:?} while evaluating kernel{kernel}",
                instance.as_raw(),
                signal
            ),
            Self::MissingSignalItemMapping {
                instance,
                kernel,
                item,
            } => write!(
                f,
                "source instance {} could not map backend item {item} to a runtime signal while evaluating kernel{kernel}",
                instance.as_raw()
            ),
            Self::EvaluateDerivedSignal {
                signal,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate derived signal {:?} for item {item}: {error}",
                signal
            ),
            Self::EvaluateSourceArgument {
                instance,
                index,
                error,
            } => write!(
                f,
                "failed to evaluate source argument {index} for instance {}: {error}",
                instance.as_raw()
            ),
            Self::EvaluateSourceOption {
                instance,
                option_name,
                error,
            } => write!(
                f,
                "failed to evaluate source option {option_name} for instance {}: {error}",
                instance.as_raw()
            ),
            Self::InvalidActiveWhenValue { instance, value } => write!(
                f,
                "source instance {} produced non-Bool activeWhen value {:?}",
                instance.as_raw(),
                value
            ),
        }
    }
}

impl std::error::Error for BackendRuntimeError {}

struct LinkedDerivedEvaluator<'a> {
    backend: &'a BackendProgram,
    derived_signals: &'a BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    committed_signals: &'a BTreeMap<BackendItemId, RuntimeValue>,
}

impl TryDerivedNodeEvaluator<RuntimeValue> for LinkedDerivedEvaluator<'_> {
    type Error = BackendRuntimeError;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<Option<RuntimeValue>, Self::Error> {
        let binding = self
            .derived_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownDerivedSignal { signal })?;
        if inputs.len() != binding.dependency_items.len() {
            return Err(BackendRuntimeError::DerivedDependencyArityMismatch {
                signal,
                expected: binding.dependency_items.len(),
                found: inputs.len(),
            });
        }

        let mut globals = self.committed_signals.clone();
        for (index, dependency) in binding.dependency_items.iter().copied().enumerate() {
            let Some(value) = inputs.value(index) else {
                return Ok(None);
            };
            globals.insert(dependency, RuntimeValue::Signal(Box::new(value.clone())));
        }

        let mut evaluator = KernelEvaluator::new(self.backend);
        evaluator
            .evaluate_item(binding.backend_item, &globals)
            .map(Some)
            .map_err(|error| BackendRuntimeError::EvaluateDerivedSignal {
                signal,
                item: binding.item,
                error,
            })
    }
}

struct LinkArtifacts {
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
}

struct LinkBuilder<'a> {
    assembly: &'a HirRuntimeAssembly,
    core: &'a core::Module,
    backend: &'a BackendProgram,
    errors: Vec<BackendRuntimeLinkError>,
    core_to_hir: BTreeMap<core::ItemId, hir::ItemId>,
    hir_to_backend: BTreeMap<hir::ItemId, BackendItemId>,
    backend_to_hir: BTreeMap<BackendItemId, hir::ItemId>,
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
}

impl<'a> LinkBuilder<'a> {
    fn new(
        assembly: &'a HirRuntimeAssembly,
        core: &'a core::Module,
        backend: &'a BackendProgram,
    ) -> Self {
        Self {
            assembly,
            core,
            backend,
            errors: Vec::new(),
            core_to_hir: BTreeMap::new(),
            hir_to_backend: BTreeMap::new(),
            backend_to_hir: BTreeMap::new(),
            signal_items_by_handle: BTreeMap::new(),
            runtime_signal_by_item: BTreeMap::new(),
            derived_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
        }
    }

    fn build(&mut self) -> Result<LinkArtifacts, BackendRuntimeLinkErrors> {
        self.index_origins();
        self.index_signal_items();
        self.link_sources();
        self.link_derived_signals();
        if self.errors.is_empty() {
            Ok(LinkArtifacts {
                signal_items_by_handle: std::mem::take(&mut self.signal_items_by_handle),
                runtime_signal_by_item: std::mem::take(&mut self.runtime_signal_by_item),
                derived_signals: std::mem::take(&mut self.derived_signals),
                source_bindings: std::mem::take(&mut self.source_bindings),
            })
        } else {
            Err(BackendRuntimeLinkErrors::new(std::mem::take(
                &mut self.errors,
            )))
        }
    }

    fn index_origins(&mut self) {
        for (core_id, item) in self.core.items().iter() {
            self.core_to_hir.insert(core_id, item.origin);
        }
        for (backend_item, item) in self.backend.items().iter() {
            let Some(&hir_item) = self.core_to_hir.get(&item.origin) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingCoreItemOrigin {
                        backend_item,
                        core_item: item.origin,
                    });
                continue;
            };
            if let Some(previous) = self.hir_to_backend.insert(hir_item, backend_item) {
                self.errors
                    .push(BackendRuntimeLinkError::DuplicateBackendOrigin {
                        item: hir_item,
                        first: previous,
                        second: backend_item,
                    });
            }
            self.backend_to_hir.insert(backend_item, hir_item);
        }
    }

    fn index_signal_items(&mut self) {
        for binding in self.assembly.signals() {
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            self.signal_items_by_handle
                .insert(binding.signal(), backend_item);
            self.runtime_signal_by_item
                .insert(backend_item, binding.signal());
        }
    }

    fn link_sources(&mut self) {
        for source in self.assembly.sources() {
            let Some(&backend_owner) = self.hir_to_backend.get(&source.owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let Some(owner_handle) = self
                .assembly
                .owner(source.owner)
                .map(|binding| binding.handle)
            else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingRuntimeOwner {
                        owner: source.owner,
                    });
                continue;
            };
            let Some(backend_item) = self.backend.items().get(backend_owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let BackendItemKind::Signal(info) = &backend_item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let Some(backend_source_id) = info.source else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendSource {
                        owner: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let backend_source = &self.backend.sources()[backend_source_id];
            if backend_source.instance.as_raw() != source.spec.instance.as_raw() {
                self.errors
                    .push(BackendRuntimeLinkError::SourceInstanceMismatch {
                        owner: source.owner,
                        runtime: source.spec.instance,
                        backend: backend_source.instance,
                    });
                continue;
            }

            let arguments = backend_source
                .arguments
                .iter()
                .map(|argument| LinkedSourceArgument {
                    kernel: argument.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, argument.kernel),
                })
                .collect::<Vec<_>>();
            let options = backend_source
                .options
                .iter()
                .map(|option| LinkedSourceOption {
                    option_name: option.option_name.clone(),
                    kernel: option.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, option.kernel),
                })
                .collect::<Vec<_>>();

            self.source_bindings.insert(
                source.spec.instance,
                LinkedSourceBinding {
                    owner: source.owner,
                    owner_handle,
                    signal: source.signal,
                    input: source.input,
                    instance: source.spec.instance,
                    backend_owner,
                    backend_source: backend_source_id,
                    arguments: arguments.into_boxed_slice(),
                    options: options.into_boxed_slice(),
                },
            );
        }
    }

    fn link_derived_signals(&mut self) {
        for binding in self.assembly.signals() {
            let Some(derived) = binding.derived() else {
                continue;
            };
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            if binding.source_input.is_some() {
                self.errors.push(
                    BackendRuntimeLinkError::SourceBackedBodySignalNotYetLinked {
                        item: binding.item,
                    },
                );
                continue;
            }

            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(info) = &item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };
            if !item.pipelines.is_empty() {
                self.errors
                    .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                        item: binding.item,
                        count: item.pipelines.len(),
                    });
                continue;
            }
            let Some(body) = item.body else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingSignalBody {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };

            let required = self.collect_required_signal_items(binding.item, body);
            let declared = info.dependencies.clone().into_boxed_slice();
            if !same_items(&required, &declared) {
                self.errors
                    .push(BackendRuntimeLinkError::SignalRequirementMismatch {
                        item: binding.item,
                        declared,
                        required,
                    });
                continue;
            }

            let backend_dependencies = info
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    self.runtime_signal_for_backend_item(binding.item, *dependency)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            if backend_dependencies.as_ref() != binding.dependencies() {
                self.errors
                    .push(BackendRuntimeLinkError::SignalDependencyMismatch {
                        item: binding.item,
                        runtime: binding.dependencies().to_vec().into_boxed_slice(),
                        backend: backend_dependencies,
                    });
                continue;
            }

            self.derived_signals.insert(
                derived,
                LinkedDerivedSignal {
                    item: binding.item,
                    signal: derived,
                    backend_item,
                    dependency_items: info.dependencies.clone().into_boxed_slice(),
                },
            );
        }
    }

    fn runtime_signal_for_backend_item(
        &mut self,
        owner: hir::ItemId,
        dependency: BackendItemId,
    ) -> Option<SignalHandle> {
        if let Some(signal) = self.runtime_signal_by_item.get(&dependency).copied() {
            return Some(signal);
        }
        self.errors
            .push(BackendRuntimeLinkError::MissingRuntimeSignalDependency { owner, dependency });
        None
    }

    fn collect_required_signal_items(
        &mut self,
        owner: hir::ItemId,
        root: KernelId,
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernels = vec![root];
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernels.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for item_id in &kernel.global_items {
                if !visited_items.insert(*item_id) {
                    continue;
                }
                let item = &self.backend.items()[*item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(*item_id);
                    }
                    _ => {
                        let Some(body) = item.body else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: *item_id,
                                });
                            continue;
                        };
                        kernels.push(body);
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }
}

fn same_items(left: &[BackendItemId], right: &[BackendItemId]) -> bool {
    left.len() == right.len()
        && left.iter().copied().collect::<BTreeSet<_>>()
            == right.iter().copied().collect::<BTreeSet<_>>()
}

fn active_when_value(
    instance: SourceInstanceId,
    value: Option<&RuntimeValue>,
) -> Result<bool, BackendRuntimeError> {
    match value {
        None => Ok(false),
        Some(RuntimeValue::Bool(value)) => Ok(*value),
        Some(RuntimeValue::Signal(value)) => match value.as_ref() {
            RuntimeValue::Bool(value) => Ok(*value),
            other => Err(BackendRuntimeError::InvalidActiveWhenValue {
                instance,
                value: other.clone(),
            }),
        },
        Some(other) => Err(BackendRuntimeError::InvalidActiveWhenValue {
            instance,
            value: other.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module as lower_hir_module};
    use aivi_lambda::lower_module as lower_lambda_module;
    use aivi_syntax::parse_module;

    use super::*;

    struct LoweredStack {
        hir: hir::LoweringResult,
        core: core::Module,
        backend: BackendProgram,
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
        let core = core::lower_module(hir.module()).expect("typed-core lowering should succeed");
        let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
        let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
        LoweredStack { hir, core, backend }
    }

    fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
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

    #[test]
    fn linked_runtime_ticks_simple_signals_and_evaluates_source_config() {
        let lowered = lower_text(
            "runtime-startup-basic.aivi",
            r#"
val prefix = "https://example.com/"

sig id = 7
sig next = id + 1
sig enabled = True

@source http.get "{prefix}{id}" with {
    activeWhen: enabled
}
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");

        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let next_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "next"))
            .expect("next signal binding should exist")
            .signal();
        let id_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "id"))
            .expect("id signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(id_signal).unwrap(),
            Some(&RuntimeValue::Int(7))
        );
        assert_eq!(
            linked.runtime().current_value(next_signal).unwrap(),
            Some(&RuntimeValue::Int(8))
        );
        assert_eq!(outcome.source_actions().len(), 1);
        let action = &outcome.source_actions()[0];
        assert_eq!(action.kind(), SourceLifecycleActionKind::Activate);
        let config = action.config().expect("activation should carry config");
        assert_eq!(
            config.arguments.as_ref(),
            &[RuntimeValue::Text("https://example.com/7".into())]
        );
        assert_eq!(config.options.len(), 1);
        assert_eq!(config.options[0].option_name.as_ref(), "activeWhen");
        assert_eq!(
            config.options[0].value,
            RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true)))
        );
    }

    #[test]
    fn linked_runtime_reports_missing_signal_snapshots_for_source_config() {
        let lowered = lower_text(
            "runtime-startup-missing-snapshot.aivi",
            r#"
@source http.get "/host"
sig apiHost : Signal Text

@source http.get "{apiHost}/users"
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let users = item_id(lowered.hir.module(), "users");
        let instance = linked
            .source_by_owner(users)
            .expect("users source binding should exist")
            .instance;
        let error = linked
            .evaluate_source_config(instance)
            .expect_err("missing signal snapshots should be reported");
        assert!(matches!(
            error,
            BackendRuntimeError::MissingCommittedSignalSnapshot { instance: found, .. } if found == instance
        ));
    }

    #[test]
    fn linked_runtime_reports_missing_signal_item_mappings_for_source_config() {
        let lowered = lower_text(
            "runtime-startup-missing-signal-mapping.aivi",
            r#"
@source http.get "/host"
sig apiHost : Signal Text

@source http.get "{apiHost}/users"
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let users = item_id(lowered.hir.module(), "users");
        let binding = linked
            .source_by_owner(users)
            .expect("users source binding should exist")
            .clone();
        let required_item = binding.arguments[0].required_signals[0];
        linked.runtime_signal_by_item.remove(&required_item);

        let error = linked
            .evaluate_source_config(binding.instance)
            .expect_err("missing signal-item mappings should be reported explicitly");
        assert!(matches!(
            error,
            BackendRuntimeError::MissingSignalItemMapping {
                instance,
                item,
                ..
            } if instance == binding.instance && item == required_item
        ));
    }

    #[test]
    fn linked_runtime_evaluates_helper_kernels_with_inline_case_pipes() {
        let lowered = lower_text(
            "runtime-startup-inline-case-helper.aivi",
            r#"
fun choose:Text #maybeName:(Option Text) =>
    maybeName
     ||> Some name => name
     ||> None => "guest"

sig maybeName = Some "Ada"
sig label = choose maybeName
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(label_signal).unwrap(),
            Some(&RuntimeValue::Text("Ada".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_rejects_source_backed_body_signals() {
        let lowered = lower_text(
            "runtime-startup-source-body-gap.aivi",
            r#"
fun step:Int #value:Int =>
    value

sig enabled = True

@source http.get "/users" with {
    activeWhen: enabled
}
sig gated : Signal Int =
    0
     @|> step
     <|@ step
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let errors = match link_backend_runtime(assembly, &lowered.core, &lowered.backend) {
            Ok(_) => panic!("source-backed body signals should stay an explicit startup gap"),
            Err(errors) => errors,
        };
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            BackendRuntimeLinkError::SourceBackedBodySignalNotYetLinked { item }
                if *item == item_id(lowered.hir.module(), "gated")
        )));
    }
}
