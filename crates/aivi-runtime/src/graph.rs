use std::collections::{BTreeSet, VecDeque};

macro_rules! define_handle {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub const fn as_raw(self) -> u32 {
                self.0
            }

            /// # Invariant
            ///
            /// The caller must ensure `raw` was obtained from a valid `Handle::as_raw()` call
            /// on a handle that was created from a live graph, and the graph has not been
            /// cleared or rebuilt since. Violating this invariant will silently reference the
            /// wrong graph node.
            pub(crate) const fn from_raw(raw: u32) -> Self {
                Self(raw)
            }

            pub(crate) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_handle!(SignalHandle);
define_handle!(OwnerHandle);
define_handle!(ReactiveClauseHandle);

impl SignalHandle {
    pub const fn as_input(self) -> InputHandle {
        InputHandle(self)
    }

    pub const fn as_derived(self) -> DerivedHandle {
        DerivedHandle(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputHandle(SignalHandle);

impl InputHandle {
    pub const fn as_signal(self) -> SignalHandle {
        self.0
    }

    pub const fn as_raw(self) -> u32 {
        self.0.as_raw()
    }

    /// # Invariant
    ///
    /// The caller must ensure `raw` was obtained from a valid `InputHandle::as_raw()` call
    /// on a handle that was created from a live graph, and the graph has not been cleared
    /// or rebuilt since. Violating this invariant will silently reference the wrong node.
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(SignalHandle::from_raw(raw))
    }

    pub(crate) const fn index(self) -> usize {
        self.0.index()
    }
}

impl From<InputHandle> for SignalHandle {
    fn from(value: InputHandle) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DerivedHandle(SignalHandle);

impl DerivedHandle {
    pub const fn as_signal(self) -> SignalHandle {
        self.0
    }

    pub const fn as_raw(self) -> u32 {
        self.0.as_raw()
    }

    /// # Invariant
    ///
    /// The caller must ensure `raw` was obtained from a valid `DerivedHandle::as_raw()` call
    /// on a handle that was created from a live graph, and the graph has not been cleared
    /// or rebuilt since. Violating this invariant will silently reference the wrong node.
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(SignalHandle::from_raw(raw))
    }

    pub(crate) const fn index(self) -> usize {
        self.0.index()
    }
}

impl From<DerivedHandle> for SignalHandle {
    fn from(value: DerivedHandle) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalGraph {
    owners: Vec<OwnerSpec>,
    signals: Vec<SignalSpec>,
    batches: Vec<TopologyBatch>,
    dependents: Vec<Box<[SignalHandle]>>,
    reactive_clauses: Vec<ReactiveClauseSpec>,
}

impl SignalGraph {
    pub fn builder() -> SignalGraphBuilder {
        SignalGraphBuilder::new()
    }

    pub fn signals(&self) -> impl ExactSizeIterator<Item = (SignalHandle, &SignalSpec)> {
        self.signals
            .iter()
            .enumerate()
            .map(|(index, spec)| (SignalHandle::from_raw(index as u32), spec))
    }

    pub fn owners(&self) -> impl ExactSizeIterator<Item = (OwnerHandle, &OwnerSpec)> {
        self.owners
            .iter()
            .enumerate()
            .map(|(index, spec)| (OwnerHandle::from_raw(index as u32), spec))
    }

    pub fn signal_count(&self) -> usize {
        self.signals.len()
    }

    pub fn owner_count(&self) -> usize {
        self.owners.len()
    }

    pub fn signal(&self, handle: SignalHandle) -> Option<&SignalSpec> {
        self.signals.get(handle.index())
    }

    pub fn owner(&self, handle: OwnerHandle) -> Option<&OwnerSpec> {
        self.owners.get(handle.index())
    }

    pub fn derived(&self, handle: DerivedHandle) -> Option<&DerivedSpec> {
        self.signal(handle.as_signal())?.kind().as_derived()
    }

    pub fn reactive(&self, handle: SignalHandle) -> Option<&ReactiveSignalSpec> {
        self.signal(handle)?.kind().as_reactive()
    }

    pub fn dependencies(&self, handle: DerivedHandle) -> Option<&[SignalHandle]> {
        Some(self.derived(handle)?.dependencies())
    }

    pub fn signal_dependencies(&self, handle: SignalHandle) -> Option<&[SignalHandle]> {
        let spec = self.signal(handle)?;
        Some(match spec.kind() {
            SignalKind::Input => &[],
            SignalKind::Derived(spec) => spec.dependencies(),
            SignalKind::Reactive(spec) => spec.dependencies(),
        })
    }

    pub fn dependents(&self, handle: SignalHandle) -> Option<&[SignalHandle]> {
        Some(self.dependents.get(handle.index())?)
    }

    pub fn batches(&self) -> &[TopologyBatch] {
        &self.batches
    }

    pub fn reactive_clause(&self, handle: ReactiveClauseHandle) -> Option<&ReactiveClauseSpec> {
        self.reactive_clauses.get(handle.index())
    }

    pub(crate) fn contains_signal(&self, handle: SignalHandle) -> bool {
        handle.index() < self.signals.len()
    }

    pub(crate) fn contains_owner(&self, handle: OwnerHandle) -> bool {
        handle.index() < self.owners.len()
    }

    /// Validate that `input` refers to an existing input signal in this graph.
    ///
    /// Returns `Ok(())` only when the handle is in-bounds and the signal at that index has kind
    /// [`SignalKind::Input`].  All other conditions (out-of-bounds raw index, or the slot holds a
    /// derived signal) are reported as distinct [`InputValidationError`] variants so callers can
    /// surface precise diagnostics rather than silently dispatching to the wrong node.
    pub fn validate_input(&self, input: InputHandle) -> Result<(), InputValidationError> {
        match self.signal(input.as_signal()).map(|s| s.kind()) {
            Some(SignalKind::Input) => Ok(()),
            Some(SignalKind::Derived(_)) | Some(SignalKind::Reactive(_)) => {
                Err(InputValidationError::NotAnInput {
                raw: input.as_raw(),
                })
            }
            None => Err(InputValidationError::UnknownHandle {
                raw: input.as_raw(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerSpec {
    name: Box<str>,
    parent: Option<OwnerHandle>,
    children: Box<[OwnerHandle]>,
    signals: Box<[SignalHandle]>,
}

impl OwnerSpec {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parent(&self) -> Option<OwnerHandle> {
        self.parent
    }

    pub fn children(&self) -> &[OwnerHandle] {
        &self.children
    }

    pub fn signals(&self) -> &[SignalHandle] {
        &self.signals
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalSpec {
    name: Box<str>,
    owner: Option<OwnerHandle>,
    kind: SignalKind,
}

impl SignalSpec {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn owner(&self) -> Option<OwnerHandle> {
        self.owner
    }

    pub fn kind(&self) -> &SignalKind {
        &self.kind
    }

    pub fn is_input(&self) -> bool {
        matches!(self.kind, SignalKind::Input)
    }

    pub fn is_derived(&self) -> bool {
        matches!(self.kind, SignalKind::Derived(_))
    }

    pub fn is_reactive(&self) -> bool {
        matches!(self.kind, SignalKind::Reactive(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalKind {
    Input,
    Derived(DerivedSpec),
    Reactive(ReactiveSignalSpec),
}

impl SignalKind {
    pub fn as_derived(&self) -> Option<&DerivedSpec> {
        match self {
            Self::Input => None,
            Self::Derived(spec) => Some(spec),
            Self::Reactive(_) => None,
        }
    }

    pub fn as_reactive(&self) -> Option<&ReactiveSignalSpec> {
        match self {
            Self::Input | Self::Derived(_) => None,
            Self::Reactive(spec) => Some(spec),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedSpec {
    dependencies: Box<[SignalHandle]>,
    batch: usize,
}

impl DerivedSpec {
    pub fn dependencies(&self) -> &[SignalHandle] {
        &self.dependencies
    }

    pub fn batch(&self) -> usize {
        self.batch
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveSignalSpec {
    dependencies: Box<[SignalHandle]>,
    seed_dependencies: Box<[SignalHandle]>,
    clauses: Box<[ReactiveClauseHandle]>,
    batch: usize,
}

impl ReactiveSignalSpec {
    pub fn dependencies(&self) -> &[SignalHandle] {
        &self.dependencies
    }

    pub fn seed_dependencies(&self) -> &[SignalHandle] {
        &self.seed_dependencies
    }

    pub fn clauses(&self) -> &[ReactiveClauseHandle] {
        &self.clauses
    }

    pub fn batch(&self) -> usize {
        self.batch
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveClauseSpec {
    target: SignalHandle,
    source_order: usize,
    guard_dependencies: Box<[SignalHandle]>,
    body_dependencies: Box<[SignalHandle]>,
}

impl ReactiveClauseSpec {
    pub fn target(&self) -> SignalHandle {
        self.target
    }

    pub fn source_order(&self) -> usize {
        self.source_order
    }

    pub fn guard_dependencies(&self) -> &[SignalHandle] {
        &self.guard_dependencies
    }

    pub fn body_dependencies(&self) -> &[SignalHandle] {
        &self.body_dependencies
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyBatch {
    index: usize,
    signals: Box<[SignalHandle]>,
}

impl TopologyBatch {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn signals(&self) -> &[SignalHandle] {
        &self.signals
    }
}

/// Error returned by [`SignalGraph::validate_input`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputValidationError {
    /// The raw handle index is out of bounds for this graph.
    UnknownHandle { raw: u32 },
    /// The handle is in-bounds but refers to a derived signal, not an input.
    NotAnInput { raw: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphBuildError {
    UnknownOwnerHandle {
        owner: u32,
    },
    UnknownSignalHandle {
        signal: u32,
    },
    SignalIsNotDerived {
        signal: u32,
    },
    SignalIsNotReactive {
        signal: u32,
    },
    DerivedAlreadyDefined {
        signal: DerivedHandle,
    },
    ReactiveAlreadyDefined {
        signal: SignalHandle,
    },
    UndefinedDerivedSignals {
        signals: Vec<DerivedHandle>,
    },
    DuplicateDependency {
        signal: DerivedHandle,
        dependency: SignalHandle,
    },
    SelfDependency {
        signal: DerivedHandle,
    },
    DependencyCycle {
        signals: Vec<SignalHandle>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct SignalGraphBuilder {
    owners: Vec<OwnerBuilderEntry>,
    signals: Vec<SignalBuilderEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveClauseBuilderSpec {
    pub guard_dependencies: Box<[SignalHandle]>,
    pub body_dependencies: Box<[SignalHandle]>,
}

impl ReactiveClauseBuilderSpec {
    pub fn new(
        guard_dependencies: impl IntoIterator<Item = SignalHandle>,
        body_dependencies: impl IntoIterator<Item = SignalHandle>,
    ) -> Self {
        Self {
            guard_dependencies: guard_dependencies.into_iter().collect(),
            body_dependencies: body_dependencies.into_iter().collect(),
        }
    }
}

impl SignalGraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_owner(
        &mut self,
        name: impl Into<Box<str>>,
        parent: Option<OwnerHandle>,
    ) -> Result<OwnerHandle, GraphBuildError> {
        if let Some(parent) = parent {
            self.validate_owner(parent)?;
        }

        let handle = OwnerHandle::from_raw(self.owners.len() as u32);
        self.owners.push(OwnerBuilderEntry {
            name: name.into(),
            parent,
            children: Vec::new(),
            signals: Vec::new(),
        });

        if let Some(parent) = parent {
            self.owners[parent.index()].children.push(handle);
        }

        Ok(handle)
    }

    pub fn add_input(
        &mut self,
        name: impl Into<Box<str>>,
        owner: Option<OwnerHandle>,
    ) -> Result<InputHandle, GraphBuildError> {
        if let Some(owner) = owner {
            self.validate_owner(owner)?;
        }

        let handle = InputHandle::from_raw(self.signals.len() as u32);
        self.signals.push(SignalBuilderEntry {
            name: name.into(),
            owner,
            kind: PendingSignalKind::Input,
        });

        if let Some(owner) = owner {
            self.owners[owner.index()].signals.push(handle.as_signal());
        }

        Ok(handle)
    }

    pub fn add_derived(
        &mut self,
        name: impl Into<Box<str>>,
        owner: Option<OwnerHandle>,
    ) -> Result<DerivedHandle, GraphBuildError> {
        if let Some(owner) = owner {
            self.validate_owner(owner)?;
        }

        let handle = DerivedHandle::from_raw(self.signals.len() as u32);
        self.signals.push(SignalBuilderEntry {
            name: name.into(),
            owner,
            kind: PendingSignalKind::Derived { dependencies: None },
        });

        if let Some(owner) = owner {
            self.owners[owner.index()].signals.push(handle.as_signal());
        }

        Ok(handle)
    }

    pub fn add_reactive(
        &mut self,
        name: impl Into<Box<str>>,
        owner: Option<OwnerHandle>,
    ) -> Result<SignalHandle, GraphBuildError> {
        if let Some(owner) = owner {
            self.validate_owner(owner)?;
        }

        let handle = SignalHandle::from_raw(self.signals.len() as u32);
        self.signals.push(SignalBuilderEntry {
            name: name.into(),
            owner,
            kind: PendingSignalKind::Reactive { spec: None },
        });

        if let Some(owner) = owner {
            self.owners[owner.index()].signals.push(handle);
        }

        Ok(handle)
    }

    pub fn define_derived(
        &mut self,
        signal: DerivedHandle,
        dependencies: impl IntoIterator<Item = SignalHandle>,
    ) -> Result<(), GraphBuildError> {
        let Some(entry) = self.signals.get(signal.index()) else {
            return Err(GraphBuildError::UnknownSignalHandle {
                signal: signal.as_raw(),
            });
        };

        let PendingSignalKind::Derived { dependencies: slot } = &entry.kind else {
            return Err(GraphBuildError::SignalIsNotDerived {
                signal: signal.as_raw(),
            });
        };

        if slot.is_some() {
            return Err(GraphBuildError::DerivedAlreadyDefined { signal });
        }

        let mut seen = BTreeSet::new();
        let mut collected = Vec::new();

        for dependency in dependencies {
            self.validate_signal(dependency)?;
            if dependency == signal.as_signal() {
                return Err(GraphBuildError::SelfDependency { signal });
            }
            if !seen.insert(dependency.as_raw()) {
                return Err(GraphBuildError::DuplicateDependency { signal, dependency });
            }
            collected.push(dependency);
        }

        let PendingSignalKind::Derived { dependencies: slot } =
            &mut self.signals[signal.index()].kind
        else {
            unreachable!("validated derived signal kind changed unexpectedly");
        };
        *slot = Some(collected.into_boxed_slice());
        Ok(())
    }

    pub fn define_reactive(
        &mut self,
        signal: SignalHandle,
        seed_dependencies: impl IntoIterator<Item = SignalHandle>,
        clauses: impl IntoIterator<Item = ReactiveClauseBuilderSpec>,
    ) -> Result<(), GraphBuildError> {
        let Some(entry) = self.signals.get(signal.index()) else {
            return Err(GraphBuildError::UnknownSignalHandle {
                signal: signal.as_raw(),
            });
        };

        let PendingSignalKind::Reactive { spec } = &entry.kind else {
            return Err(GraphBuildError::SignalIsNotReactive {
                signal: signal.as_raw(),
            });
        };

        if spec.is_some() {
            return Err(GraphBuildError::ReactiveAlreadyDefined { signal });
        }

        let mut merged = BTreeSet::new();
        let seed_dependencies = seed_dependencies
            .into_iter()
            .map(|dependency| {
                self.validate_signal(dependency)?;
                if dependency == signal {
                    return Err(GraphBuildError::SelfDependency {
                        signal: signal.as_derived(),
                    });
                }
                merged.insert(dependency);
                Ok(dependency)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let clauses = clauses
            .into_iter()
            .map(|clause| {
                for dependency in clause
                    .guard_dependencies
                    .iter()
                    .chain(clause.body_dependencies.iter())
                    .copied()
                {
                    self.validate_signal(dependency)?;
                    if dependency == signal {
                        return Err(GraphBuildError::SelfDependency {
                            signal: signal.as_derived(),
                        });
                    }
                    merged.insert(dependency);
                }
                Ok(PendingReactiveClause {
                    guard_dependencies: clause.guard_dependencies,
                    body_dependencies: clause.body_dependencies,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let PendingSignalKind::Reactive { spec } = &mut self.signals[signal.index()].kind else {
            unreachable!("validated reactive signal kind changed unexpectedly");
        };
        *spec = Some(PendingReactiveSpec {
            dependencies: merged.into_iter().collect(),
            seed_dependencies: seed_dependencies.into_boxed_slice(),
            clauses,
        });
        Ok(())
    }

    pub fn build(self) -> Result<SignalGraph, GraphBuildError> {
        let signal_count = self.signals.len() as u32;
        let undefined = self
            .signals
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry.kind {
                PendingSignalKind::Input => None,
                PendingSignalKind::Derived { dependencies: None } => {
                    debug_assert!(
                        (index as u32) < signal_count,
                        "Handle::from_raw: raw index {} out of bounds (signals len {})",
                        index,
                        signal_count
                    );
                    Some(DerivedHandle::from_raw(index as u32))
                }
                PendingSignalKind::Derived {
                    dependencies: Some(_),
                } => None,
                PendingSignalKind::Reactive { spec: None } => {
                    debug_assert!(
                        (index as u32) < signal_count,
                        "Handle::from_raw: raw index {} out of bounds (signals len {})",
                        index,
                        signal_count
                    );
                    Some(SignalHandle::from_raw(index as u32).as_derived())
                }
                PendingSignalKind::Reactive { spec: Some(_) } => None,
            })
            .collect::<Vec<_>>();
        if !undefined.is_empty() {
            return Err(GraphBuildError::UndefinedDerivedSignals { signals: undefined });
        }

        let mut signals = Vec::with_capacity(self.signals.len());
        let mut reactive_clauses = Vec::new();
        let mut dependents = (0..self.signals.len())
            .map(|_| Vec::new())
            .collect::<Vec<Vec<SignalHandle>>>();
        let mut indegree = vec![0usize; self.signals.len()];
        let mut signal_batches = vec![0usize; self.signals.len()];

        for (index, entry) in self.signals.into_iter().enumerate() {
            let kind = match entry.kind {
                PendingSignalKind::Input => SignalKind::Input,
                PendingSignalKind::Derived {
                    dependencies: Some(dependencies),
                } => {
                    indegree[index] = dependencies.len();
                    debug_assert!(
                        (index as u32) < signal_count,
                        "Handle::from_raw: raw index {} out of bounds (signals len {})",
                        index,
                        signal_count
                    );
                    let derived = DerivedHandle::from_raw(index as u32);
                    for &dependency in &dependencies {
                        dependents[dependency.index()].push(derived.as_signal());
                    }
                    SignalKind::Derived(DerivedSpec {
                        dependencies,
                        batch: 0,
                    })
                }
                PendingSignalKind::Derived { dependencies: None } => unreachable!(),
                PendingSignalKind::Reactive { spec: Some(spec) } => {
                    indegree[index] = spec.dependencies.len();
                    let signal = SignalHandle::from_raw(index as u32);
                    for &dependency in &spec.dependencies {
                        dependents[dependency.index()].push(signal);
                    }
                    let clause_handles = spec
                        .clauses
                        .into_iter()
                        .enumerate()
                        .map(|(source_order, clause)| {
                            let handle = ReactiveClauseHandle::from_raw(reactive_clauses.len() as u32);
                            reactive_clauses.push(ReactiveClauseSpec {
                                target: signal,
                                source_order,
                                guard_dependencies: clause.guard_dependencies,
                                body_dependencies: clause.body_dependencies,
                            });
                            handle
                        })
                        .collect::<Vec<_>>();
                    SignalKind::Reactive(ReactiveSignalSpec {
                        dependencies: spec.dependencies.into_boxed_slice(),
                        seed_dependencies: spec.seed_dependencies,
                        clauses: clause_handles.into_boxed_slice(),
                        batch: 0,
                    })
                }
                PendingSignalKind::Reactive { spec: None } => unreachable!(),
            };

            signals.push(SignalSpec {
                name: entry.name,
                owner: entry.owner,
                kind,
            });
        }

        for dependents in &mut dependents {
            dependents.sort_by_key(|signal| signal.as_raw());
        }

        let mut ready = (0..signals.len())
            .filter(|&index| indegree[index] == 0)
            .collect::<VecDeque<_>>();
        let mut processed = 0usize;
        while let Some(index) = ready.pop_front() {
            processed += 1;
            let next_batch = match signals[index].kind {
                SignalKind::Input => 0,
                SignalKind::Derived(_) | SignalKind::Reactive(_) => signal_batches[index] + 1,
            };

            for &dependent in &dependents[index] {
                let dependent_index = dependent.index();
                signal_batches[dependent_index] = signal_batches[dependent_index].max(next_batch);
                indegree[dependent_index] -= 1;
                if indegree[dependent_index] == 0 {
                    ready.push_back(dependent_index);
                }
            }
        }

        if processed != signals.len() {
            let cycle = indegree
                .iter()
                .enumerate()
                .filter_map(|(index, remaining)| {
                    (*remaining > 0).then(|| {
                        debug_assert!(
                            (index as u32) < signals.len() as u32,
                            "Handle::from_raw: raw index {} out of bounds (signals len {})",
                            index,
                            signals.len()
                        );
                        SignalHandle::from_raw(index as u32)
                    })
                })
                .collect::<Vec<_>>();
            return Err(GraphBuildError::DependencyCycle { signals: cycle });
        }

        let mut batches = Vec::<Vec<SignalHandle>>::new();
        for (index, signal) in signals.iter_mut().enumerate() {
            let batch = match &mut signal.kind {
                SignalKind::Input => continue,
                SignalKind::Derived(spec) => {
                    spec.batch = signal_batches[index];
                    spec.batch
                }
                SignalKind::Reactive(spec) => {
                    spec.batch = signal_batches[index];
                    spec.batch
                }
            };
            if batches.len() <= batch {
                batches.resize_with(batch + 1, Vec::new);
            }
            batches[batch].push({
                debug_assert!(
                    (index as u32) < signal_count,
                    "Handle::from_raw: raw index {} out of bounds (signals len {})",
                    index,
                    signal_count
                );
                SignalHandle::from_raw(index as u32)
            });
        }
        for batch in &mut batches {
            batch.sort_by_key(|signal| signal.as_raw());
        }

        let owners = self
            .owners
            .into_iter()
            .map(|owner| OwnerSpec {
                name: owner.name,
                parent: owner.parent,
                children: owner.children.into_boxed_slice(),
                signals: owner.signals.into_boxed_slice(),
            })
            .collect();
        let batches = batches
            .into_iter()
            .enumerate()
            .map(|(index, signals)| TopologyBatch {
                index,
                signals: signals.into_boxed_slice(),
            })
            .collect();
        let dependents = dependents
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect::<Vec<_>>();

        Ok(SignalGraph {
            owners,
            signals,
            batches,
            dependents,
            reactive_clauses,
        })
    }

    fn validate_owner(&self, owner: OwnerHandle) -> Result<(), GraphBuildError> {
        if owner.index() < self.owners.len() {
            Ok(())
        } else {
            Err(GraphBuildError::UnknownOwnerHandle {
                owner: owner.as_raw(),
            })
        }
    }

    fn validate_signal(&self, signal: SignalHandle) -> Result<(), GraphBuildError> {
        if signal.index() < self.signals.len() {
            Ok(())
        } else {
            Err(GraphBuildError::UnknownSignalHandle {
                signal: signal.as_raw(),
            })
        }
    }
}

#[derive(Clone, Debug)]
struct OwnerBuilderEntry {
    name: Box<str>,
    parent: Option<OwnerHandle>,
    children: Vec<OwnerHandle>,
    signals: Vec<SignalHandle>,
}

#[derive(Clone, Debug)]
struct SignalBuilderEntry {
    name: Box<str>,
    owner: Option<OwnerHandle>,
    kind: PendingSignalKind,
}

#[derive(Clone, Debug)]
enum PendingSignalKind {
    Input,
    Derived {
        dependencies: Option<Box<[SignalHandle]>>,
    },
    Reactive {
        spec: Option<PendingReactiveSpec>,
    },
}

#[derive(Clone, Debug)]
struct PendingReactiveSpec {
    dependencies: Vec<SignalHandle>,
    seed_dependencies: Box<[SignalHandle]>,
    clauses: Vec<PendingReactiveClause>,
}

#[derive(Clone, Debug)]
struct PendingReactiveClause {
    guard_dependencies: Box<[SignalHandle]>,
    body_dependencies: Box<[SignalHandle]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_batches_respect_static_topology() {
        let mut builder = SignalGraphBuilder::new();
        let left_input = builder.add_input("left", None).unwrap();
        let right_input = builder.add_input("right", None).unwrap();
        let join = builder.add_derived("join", None).unwrap();
        let left = builder.add_derived("left-branch", None).unwrap();
        let right = builder.add_derived("right-branch", None).unwrap();

        builder
            .define_derived(join, [left.as_signal(), right.as_signal()])
            .unwrap();
        builder
            .define_derived(left, [left_input.as_signal()])
            .unwrap();
        builder
            .define_derived(right, [right_input.as_signal()])
            .unwrap();

        let graph = builder.build().unwrap();

        assert_eq!(graph.signal_count(), 5);
        assert_eq!(graph.batches().len(), 2);
        assert_eq!(
            graph.batches()[0].signals(),
            &[left.as_signal(), right.as_signal()]
        );
        assert_eq!(graph.batches()[1].signals(), &[join.as_signal()]);
        assert_eq!(
            graph.dependencies(join).unwrap(),
            &[left.as_signal(), right.as_signal()]
        );
    }

    #[test]
    fn graph_rejects_dependency_cycles() {
        let mut builder = SignalGraphBuilder::new();
        let first = builder.add_derived("first", None).unwrap();
        let second = builder.add_derived("second", None).unwrap();

        builder.define_derived(first, [second.as_signal()]).unwrap();
        builder.define_derived(second, [first.as_signal()]).unwrap();

        assert_eq!(
            builder.build(),
            Err(GraphBuildError::DependencyCycle {
                signals: vec![first.as_signal(), second.as_signal()],
            })
        );
    }

    #[test]
    fn graph_batches_reactive_signals_with_other_evaluable_nodes() {
        let mut builder = SignalGraphBuilder::new();
        let trigger = builder.add_input("trigger", None).unwrap();
        let total = builder.add_reactive("total", None).unwrap();
        let doubled = builder.add_derived("doubled", None).unwrap();

        builder
            .define_reactive(
                total,
                [],
                [ReactiveClauseBuilderSpec::new(
                    [trigger.as_signal()],
                    [trigger.as_signal()],
                )],
            )
            .unwrap();
        builder
            .define_derived(doubled, [total])
            .unwrap();

        let graph = builder.build().unwrap();
        let reactive = graph
            .reactive(total)
            .expect("reactive signal should be lowered into the graph");
        let clause = graph
            .reactive_clause(reactive.clauses()[0])
            .expect("reactive clause should be addressable");

        assert_eq!(graph.batches().len(), 2);
        assert_eq!(graph.batches()[0].signals(), &[total]);
        assert_eq!(graph.batches()[1].signals(), &[doubled.as_signal()]);
        assert_eq!(graph.dependents(trigger.as_signal()).unwrap(), &[total]);
        assert_eq!(graph.dependents(total).unwrap(), &[doubled.as_signal()]);
        assert_eq!(reactive.dependencies(), &[trigger.as_signal()]);
        assert_eq!(clause.guard_dependencies(), &[trigger.as_signal()]);
        assert_eq!(clause.body_dependencies(), &[trigger.as_signal()]);
    }
}
