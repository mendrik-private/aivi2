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
    dependents: Vec<Box<[DerivedHandle]>>,
}

impl SignalGraph {
    pub fn builder() -> SignalGraphBuilder {
        SignalGraphBuilder::new()
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

    pub fn dependencies(&self, handle: DerivedHandle) -> Option<&[SignalHandle]> {
        Some(self.derived(handle)?.dependencies())
    }

    pub fn dependents(&self, handle: SignalHandle) -> Option<&[DerivedHandle]> {
        Some(self.dependents.get(handle.index())?)
    }

    pub fn batches(&self) -> &[TopologyBatch] {
        &self.batches
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
            Some(SignalKind::Derived(_)) => Err(InputValidationError::NotAnInput {
                raw: input.as_raw(),
            }),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalKind {
    Input,
    Derived(DerivedSpec),
}

impl SignalKind {
    pub fn as_derived(&self) -> Option<&DerivedSpec> {
        match self {
            Self::Input => None,
            Self::Derived(spec) => Some(spec),
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
pub struct TopologyBatch {
    index: usize,
    signals: Box<[DerivedHandle]>,
}

impl TopologyBatch {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn signals(&self) -> &[DerivedHandle] {
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
    DerivedAlreadyDefined {
        signal: DerivedHandle,
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
            })
            .collect::<Vec<_>>();
        if !undefined.is_empty() {
            return Err(GraphBuildError::UndefinedDerivedSignals { signals: undefined });
        }

        let mut signals = Vec::with_capacity(self.signals.len());
        let mut dependents = (0..self.signals.len())
            .map(|_| Vec::new())
            .collect::<Vec<Vec<DerivedHandle>>>();
        let mut indegree = vec![0usize; self.signals.len()];
        let mut derived_batches = vec![0usize; self.signals.len()];

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
                        dependents[dependency.index()].push(derived);
                    }
                    SignalKind::Derived(DerivedSpec {
                        dependencies,
                        batch: 0,
                    })
                }
                PendingSignalKind::Derived { dependencies: None } => unreachable!(),
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
                SignalKind::Derived(_) => derived_batches[index] + 1,
            };

            for &dependent in &dependents[index] {
                let dependent_index = dependent.index();
                derived_batches[dependent_index] = derived_batches[dependent_index].max(next_batch);
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

        let mut batches = Vec::<Vec<DerivedHandle>>::new();
        for (index, signal) in signals.iter_mut().enumerate() {
            let SignalKind::Derived(spec) = &mut signal.kind else {
                continue;
            };
            spec.batch = derived_batches[index];
            if batches.len() <= spec.batch {
                batches.resize_with(spec.batch + 1, Vec::new);
            }
            batches[spec.batch].push({
                debug_assert!(
                    (index as u32) < signal_count,
                    "Handle::from_raw: raw index {} out of bounds (signals len {})",
                    index,
                    signal_count
                );
                DerivedHandle::from_raw(index as u32)
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
        assert_eq!(graph.batches()[0].signals(), &[left, right]);
        assert_eq!(graph.batches()[1].signals(), &[join]);
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
}
