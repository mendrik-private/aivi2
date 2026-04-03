//! Maps runtime handles to user-visible names and source spans for error rendering.
//!
//! Built once from [`HirRuntimeAssembly`] and the associated [`SignalGraph`], then
//! passed to the error rendering layer so that runtime failures can reference source
//! code instead of opaque internal IDs.

use std::collections::BTreeMap;

use aivi_base::SourceSpan;
use aivi_hir as hir;

use crate::{
    effects::SourceInstanceId,
    graph::{DerivedHandle, OwnerHandle, SignalGraph, SignalHandle, SignalKind},
    hir_adapter::HirRuntimeAssembly,
};

/// Information about a signal available for error rendering.
#[derive(Clone, Debug)]
pub struct RuntimeSignalInfo {
    pub name: Box<str>,
    pub span: SourceSpan,
    pub kind: RuntimeSignalKind,
    /// Pipeline IDs from the linked runtime (populated after linking).
    pub pipeline_ids: Option<Box<[aivi_backend::PipelineId]>>,
}

/// Classifies a signal for display purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeSignalKind {
    Input,
    Derived,
    Reactive,
}

impl std::fmt::Display for RuntimeSignalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input => write!(f, "input"),
            Self::Derived => write!(f, "derived"),
            Self::Reactive => write!(f, "reactive"),
        }
    }
}

/// Information about a source instance available for error rendering.
#[derive(Clone, Debug)]
pub struct RuntimeSourceInfo {
    pub name: Box<str>,
    pub span: SourceSpan,
    pub signal: SignalHandle,
}

/// Lightweight lookup table mapping runtime handles to user-visible names and
/// source spans. Built once from [`HirRuntimeAssembly`] during linking.
#[derive(Clone, Debug)]
pub struct RuntimeSourceMap {
    signals: BTreeMap<SignalHandle, RuntimeSignalInfo>,
    owners: BTreeMap<OwnerHandle, (Box<str>, SourceSpan)>,
    sources: BTreeMap<SourceInstanceId, RuntimeSourceInfo>,
    items: BTreeMap<hir::ItemId, (Box<str>, SourceSpan)>,
}

impl RuntimeSourceMap {
    /// Build the source map from the assembly and its signal graph.
    pub fn from_assembly(assembly: &HirRuntimeAssembly) -> Self {
        let graph = assembly.graph();
        let mut signals = BTreeMap::new();
        let mut items = BTreeMap::new();

        for binding in assembly.signals() {
            let handle = binding.signal();
            let kind = match graph.signal(handle).map(|s| s.kind()) {
                Some(SignalKind::Input) => RuntimeSignalKind::Input,
                Some(SignalKind::Derived(_)) => RuntimeSignalKind::Derived,
                Some(SignalKind::Reactive(_)) => RuntimeSignalKind::Reactive,
                None => continue,
            };
            signals.insert(
                handle,
                RuntimeSignalInfo {
                    name: binding.name.clone(),
                    span: binding.span,
                    kind,
                    pipeline_ids: None,
                },
            );
            items.insert(binding.item, (binding.name.clone(), binding.span));
        }

        let mut owners = BTreeMap::new();
        for binding in assembly.owners() {
            owners.insert(binding.handle, (binding.name.clone(), binding.span));
            items
                .entry(binding.item)
                .or_insert_with(|| (binding.name.clone(), binding.span));
        }

        let mut sources = BTreeMap::new();
        for binding in assembly.sources() {
            let instance_id =
                SourceInstanceId::from_raw(binding.source_instance.decorator().as_raw());
            let name = assembly
                .owner(binding.owner)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| format!("source@{}", instance_id.as_raw()).into());
            sources.insert(
                instance_id,
                RuntimeSourceInfo {
                    name,
                    span: binding.source_span,
                    signal: binding.signal,
                },
            );
        }

        Self {
            signals,
            owners,
            sources,
            items,
        }
    }

    pub fn signal_info(&self, handle: SignalHandle) -> Option<&RuntimeSignalInfo> {
        self.signals.get(&handle)
    }

    pub fn signal_name(&self, handle: SignalHandle) -> Option<&str> {
        self.signals.get(&handle).map(|i| i.name.as_ref())
    }

    pub fn signal_span(&self, handle: SignalHandle) -> Option<SourceSpan> {
        self.signals.get(&handle).map(|i| i.span)
    }

    pub fn signal_pipeline_ids(&self, handle: SignalHandle) -> Option<&[aivi_backend::PipelineId]> {
        self.signals.get(&handle).and_then(|i| i.pipeline_ids.as_deref())
    }

    pub fn derived_name(&self, handle: DerivedHandle) -> Option<&str> {
        self.signal_name(handle.as_signal())
    }

    pub fn derived_span(&self, handle: DerivedHandle) -> Option<SourceSpan> {
        self.signal_span(handle.as_signal())
    }

    pub fn owner_name(&self, handle: OwnerHandle) -> Option<&str> {
        self.owners.get(&handle).map(|(n, _)| n.as_ref())
    }

    pub fn source_info(&self, instance: SourceInstanceId) -> Option<&RuntimeSourceInfo> {
        self.sources.get(&instance)
    }

    pub fn source_name(&self, instance: SourceInstanceId) -> Option<&str> {
        self.sources.get(&instance).map(|i| i.name.as_ref())
    }

    pub fn source_span(&self, instance: SourceInstanceId) -> Option<SourceSpan> {
        self.sources.get(&instance).map(|i| i.span)
    }

    pub fn item_name(&self, item: hir::ItemId) -> Option<&str> {
        self.items.get(&item).map(|(n, _)| n.as_ref())
    }

    pub fn item_span(&self, item: hir::ItemId) -> Option<SourceSpan> {
        self.items.get(&item).map(|(_, s)| *s)
    }

    /// Enrich signal entries with pipeline IDs from the linked runtime.
    ///
    /// Call this after linking to enable pipe-stage-level error tracking.
    pub fn set_signal_pipeline_ids(
        &mut self,
        handle: SignalHandle,
        ids: Box<[aivi_backend::PipelineId]>,
    ) {
        if let Some(info) = self.signals.get_mut(&handle) {
            info.pipeline_ids = Some(ids);
        }
    }

    /// Trace the dependency chain from a signal back to its root input sources.
    ///
    /// Returns a list of paths from the target signal to each root input. Each
    /// path is ordered root-first: `[input, intermediate…, target]`.
    pub fn trace_signal_dependencies(
        &self,
        graph: &SignalGraph,
        target: SignalHandle,
    ) -> Vec<Vec<SignalHandle>> {
        let mut paths = Vec::new();
        let mut current_path = Vec::new();
        self.trace_deps_recursive(graph, target, &mut current_path, &mut paths);
        paths
    }

    fn trace_deps_recursive(
        &self,
        graph: &SignalGraph,
        handle: SignalHandle,
        current: &mut Vec<SignalHandle>,
        paths: &mut Vec<Vec<SignalHandle>>,
    ) {
        // Cycle detection.
        if current.contains(&handle) {
            return;
        }
        current.push(handle);

        let deps = graph.signal_dependencies(handle).unwrap_or(&[]);
        if deps.is_empty() {
            // Reached a root (input signal) — collect path root-first.
            let mut path = current.clone();
            path.reverse();
            paths.push(path);
        } else {
            for &dep in deps {
                self.trace_deps_recursive(graph, dep, current, paths);
            }
        }

        current.pop();
    }

    /// Format a dependency chain as a human-readable string.
    ///
    /// Produces something like:
    /// `@source httpData → derived filteredItems → derived displayList`
    pub fn format_dependency_chain(&self, chain: &[SignalHandle]) -> String {
        let mut parts = Vec::with_capacity(chain.len());
        for &handle in chain {
            let name = self.signal_name(handle).unwrap_or("?");
            let kind = self
                .signals
                .get(&handle)
                .map(|i| i.kind)
                .unwrap_or(RuntimeSignalKind::Input);
            let prefix = match kind {
                RuntimeSignalKind::Input => "@source",
                RuntimeSignalKind::Derived => "signal",
                RuntimeSignalKind::Reactive => "reactive",
            };
            parts.push(format!("{prefix} {name}"));
        }
        parts.join(" → ")
    }
}
