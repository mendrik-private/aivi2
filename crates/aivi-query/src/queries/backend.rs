use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use aivi_backend::{
    self as backend, cache::compute_program_fingerprint,
    lower_module_with_hir as lower_backend_module, validate_program,
};
use aivi_core::{
    self as core, IncludedItems, LoweredRuntimeFragment, RuntimeFragmentSpec,
    lower_runtime_fragment, lower_runtime_fragment_with_workspace, lower_runtime_module_with_items,
    lower_runtime_module_with_workspace, validate_module as validate_core_module,
};
use aivi_hir::ItemId as HirItemId;
use aivi_lambda::{
    self as lambda, lower_module as lower_lambda_module, validate_module as validate_lambda_module,
};
use rustc_hash::FxHasher;

use crate::{RootDatabase, SourceFile, workspace::Workspace};

use super::{HirModuleResult, hir_module};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StableFingerprint(u64);

impl StableFingerprint {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WholeProgramFingerprint(StableFingerprint);

impl WholeProgramFingerprint {
    pub const fn as_u64(self) -> u64 {
        self.0.as_u64()
    }

    pub const fn stable(self) -> StableFingerprint {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeFragmentFingerprint(StableFingerprint);

impl RuntimeFragmentFingerprint {
    pub const fn as_u64(self) -> u64 {
        self.0.as_u64()
    }

    pub const fn stable(self) -> StableFingerprint {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceHirModule {
    name: Box<str>,
    file: SourceFile,
    hir: Arc<HirModuleResult>,
}

impl WorkspaceHirModule {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn file(&self) -> SourceFile {
        self.file
    }

    pub fn hir(&self) -> &HirModuleResult {
        self.hir.as_ref()
    }

    pub fn hir_arc(&self) -> Arc<HirModuleResult> {
        Arc::clone(&self.hir)
    }
}

#[derive(Clone, Debug)]
pub struct WholeProgramBackendUnit {
    entry: SourceFile,
    entry_hir: Arc<HirModuleResult>,
    workspace_modules: Arc<[WorkspaceHirModule]>,
    included_items: Arc<[HirItemId]>,
    core: Arc<core::Module>,
    lambda: Arc<lambda::Module>,
    backend: Arc<backend::Program>,
    fingerprint: WholeProgramFingerprint,
}

impl WholeProgramBackendUnit {
    pub fn entry(&self) -> SourceFile {
        self.entry
    }

    pub fn entry_hir(&self) -> &HirModuleResult {
        self.entry_hir.as_ref()
    }

    pub fn entry_hir_arc(&self) -> Arc<HirModuleResult> {
        Arc::clone(&self.entry_hir)
    }

    pub fn workspace_modules(&self) -> &[WorkspaceHirModule] {
        self.workspace_modules.as_ref()
    }

    pub fn workspace_modules_arc(&self) -> Arc<[WorkspaceHirModule]> {
        Arc::clone(&self.workspace_modules)
    }

    pub fn included_items(&self) -> &[HirItemId] {
        self.included_items.as_ref()
    }

    pub fn core(&self) -> &core::Module {
        self.core.as_ref()
    }

    pub fn core_arc(&self) -> Arc<core::Module> {
        Arc::clone(&self.core)
    }

    pub fn lambda(&self) -> &lambda::Module {
        self.lambda.as_ref()
    }

    pub fn lambda_arc(&self) -> Arc<lambda::Module> {
        Arc::clone(&self.lambda)
    }

    pub fn backend(&self) -> &backend::Program {
        self.backend.as_ref()
    }

    pub fn backend_arc(&self) -> Arc<backend::Program> {
        Arc::clone(&self.backend)
    }

    pub fn fingerprint(&self) -> WholeProgramFingerprint {
        self.fingerprint
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeFragmentBackendUnit {
    entry: SourceFile,
    entry_hir: Arc<HirModuleResult>,
    fragment: Arc<RuntimeFragmentSpec>,
    core: Arc<LoweredRuntimeFragment>,
    lambda: Arc<lambda::Module>,
    backend: Arc<backend::Program>,
    fingerprint: RuntimeFragmentFingerprint,
}

impl RuntimeFragmentBackendUnit {
    pub fn entry(&self) -> SourceFile {
        self.entry
    }

    pub fn entry_hir(&self) -> &HirModuleResult {
        self.entry_hir.as_ref()
    }

    pub fn entry_hir_arc(&self) -> Arc<HirModuleResult> {
        Arc::clone(&self.entry_hir)
    }

    pub fn fragment(&self) -> &RuntimeFragmentSpec {
        self.fragment.as_ref()
    }

    pub fn fragment_arc(&self) -> Arc<RuntimeFragmentSpec> {
        Arc::clone(&self.fragment)
    }

    pub fn core(&self) -> &LoweredRuntimeFragment {
        self.core.as_ref()
    }

    pub fn core_arc(&self) -> Arc<LoweredRuntimeFragment> {
        Arc::clone(&self.core)
    }

    pub fn lambda(&self) -> &lambda::Module {
        self.lambda.as_ref()
    }

    pub fn lambda_arc(&self) -> Arc<lambda::Module> {
        Arc::clone(&self.lambda)
    }

    pub fn backend(&self) -> &backend::Program {
        self.backend.as_ref()
    }

    pub fn backend_arc(&self) -> Arc<backend::Program> {
        Arc::clone(&self.backend)
    }

    pub fn fingerprint(&self) -> RuntimeFragmentFingerprint {
        self.fingerprint
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendUnitError {
    CoreLowering(core::LoweringErrors),
    CoreValidation(core::ValidationErrors),
    LambdaLowering(lambda::LoweringErrors),
    LambdaValidation(lambda::ValidationErrors),
    BackendLowering(backend::LoweringErrors),
    BackendValidation(backend::ValidationErrors),
}

impl BackendUnitError {
    pub fn stage(&self) -> &'static str {
        match self {
            Self::CoreLowering(_) => "typed core lowering",
            Self::CoreValidation(_) => "typed core validation",
            Self::LambdaLowering(_) => "typed lambda lowering",
            Self::LambdaValidation(_) => "typed lambda validation",
            Self::BackendLowering(_) => "backend lowering",
            Self::BackendValidation(_) => "backend validation",
        }
    }
}

impl fmt::Display for BackendUnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreLowering(errors) => {
                write!(f, "failed to lower backend unit into typed core: {errors}")
            }
            Self::CoreValidation(errors) => {
                write!(f, "typed-core validation failed for backend unit: {errors}")
            }
            Self::LambdaLowering(errors) => {
                write!(
                    f,
                    "failed to lower backend unit into typed lambda: {errors}"
                )
            }
            Self::LambdaValidation(errors) => {
                write!(
                    f,
                    "typed-lambda validation failed for backend unit: {errors}"
                )
            }
            Self::BackendLowering(errors) => {
                write!(f, "failed to lower backend unit into backend IR: {errors}")
            }
            Self::BackendValidation(errors) => {
                write!(f, "backend validation failed for backend unit: {errors}")
            }
        }
    }
}

impl std::error::Error for BackendUnitError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WholeProgramUnitCacheKey {
    pub(crate) file_id: u32,
    pub(crate) included_items_key: u64,
}

#[derive(Clone)]
pub(crate) struct WholeProgramUnitCacheEntry {
    pub(crate) entry_hir: Arc<HirModuleResult>,
    pub(crate) workspace_modules: Arc<[WorkspaceHirModule]>,
    pub(crate) value: Arc<Result<Arc<WholeProgramBackendUnit>, BackendUnitError>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RuntimeFragmentUnitCacheKey {
    pub(crate) file_id: u32,
    pub(crate) fragment_key: u64,
}

#[derive(Clone)]
pub(crate) struct RuntimeFragmentUnitCacheEntry {
    pub(crate) entry_hir: Arc<HirModuleResult>,
    pub(crate) workspace_modules: Arc<[WorkspaceHirModule]>,
    pub(crate) value: Arc<Result<Arc<RuntimeFragmentBackendUnit>, BackendUnitError>>,
}

/// Collect non-stdlib workspace modules reachable from `file`, ordered so direct
/// and transitive dependencies appear before their dependents.
pub fn reachable_workspace_hir_modules(
    db: &RootDatabase,
    file: SourceFile,
) -> Arc<[WorkspaceHirModule]> {
    let entry_hir = hir_module(db, file);
    collect_workspace_hir_modules(db, file, &entry_hir)
}

/// Lower the entry module plus its reachable workspace dependencies into a
/// backend-owned runtime unit using all root items in the entry module.
pub fn whole_program_backend_unit(
    db: &RootDatabase,
    file: SourceFile,
) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError> {
    let entry_hir = hir_module(db, file);
    let included_items = entry_hir.module().root_items().iter().copied().collect();
    whole_program_backend_unit_with_items_inner(db, file, entry_hir, &included_items)
}

/// Lower the entry module plus its reachable workspace dependencies into a
/// backend-owned runtime unit using the selected entry-module items.
pub fn whole_program_backend_unit_with_items(
    db: &RootDatabase,
    file: SourceFile,
    included_items: &IncludedItems,
) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError> {
    let entry_hir = hir_module(db, file);
    whole_program_backend_unit_with_items_inner(db, file, entry_hir, included_items)
}

pub fn whole_program_backend_fingerprint(
    db: &RootDatabase,
    file: SourceFile,
) -> Result<WholeProgramFingerprint, BackendUnitError> {
    whole_program_backend_unit(db, file).map(|unit| unit.fingerprint())
}

pub fn whole_program_backend_fingerprint_with_items(
    db: &RootDatabase,
    file: SourceFile,
    included_items: &IncludedItems,
) -> Result<WholeProgramFingerprint, BackendUnitError> {
    whole_program_backend_unit_with_items(db, file, included_items).map(|unit| unit.fingerprint())
}

/// Lower a runtime fragment spec into a standalone backend-owned unit.
pub fn runtime_fragment_backend_unit(
    db: &RootDatabase,
    file: SourceFile,
    fragment: &RuntimeFragmentSpec,
) -> Result<Arc<RuntimeFragmentBackendUnit>, BackendUnitError> {
    let entry_hir = hir_module(db, file);
    let workspace_modules = collect_workspace_hir_modules(db, file, &entry_hir);
    let key = runtime_fragment_cache_key(file, fragment);
    if let Some(cached) = db.runtime_fragment_cache_entry(key)
        && Arc::ptr_eq(&cached.entry_hir, &entry_hir)
        && workspace_modules_match(&cached.workspace_modules, &workspace_modules)
    {
        return clone_cached_value(&cached.value);
    }

    let value = Arc::new(lower_runtime_fragment_backend_unit(
        file,
        entry_hir.clone(),
        workspace_modules.clone(),
        fragment,
    ));
    db.store_runtime_fragment_cache_entry(
        key,
        RuntimeFragmentUnitCacheEntry {
            entry_hir,
            workspace_modules,
            value: Arc::clone(&value),
        },
    );
    clone_cached_value(&value)
}

pub fn runtime_fragment_backend_fingerprint(
    db: &RootDatabase,
    file: SourceFile,
    fragment: &RuntimeFragmentSpec,
) -> Result<RuntimeFragmentFingerprint, BackendUnitError> {
    runtime_fragment_backend_unit(db, file, fragment).map(|unit| unit.fingerprint())
}

fn whole_program_backend_unit_with_items_inner(
    db: &RootDatabase,
    file: SourceFile,
    entry_hir: Arc<HirModuleResult>,
    included_items: &IncludedItems,
) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError> {
    let normalized_included_items = normalize_included_items(included_items);
    let workspace_modules = collect_workspace_hir_modules(db, file, &entry_hir);
    let key = whole_program_cache_key(file, &normalized_included_items);
    if let Some(cached) = db.whole_program_cache_entry(key)
        && Arc::ptr_eq(&cached.entry_hir, &entry_hir)
        && workspace_modules_match(&cached.workspace_modules, &workspace_modules)
    {
        return clone_cached_value(&cached.value);
    }

    let value = Arc::new(lower_whole_program_backend_unit(
        file,
        entry_hir.clone(),
        workspace_modules.clone(),
        normalized_included_items.clone(),
    ));
    db.store_whole_program_cache_entry(
        key,
        WholeProgramUnitCacheEntry {
            entry_hir,
            workspace_modules,
            value: Arc::clone(&value),
        },
    );
    clone_cached_value(&value)
}

fn lower_whole_program_backend_unit(
    file: SourceFile,
    entry_hir: Arc<HirModuleResult>,
    workspace_modules: Arc<[WorkspaceHirModule]>,
    normalized_included_items: Arc<[HirItemId]>,
) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError> {
    let included_items = normalized_included_items
        .iter()
        .copied()
        .collect::<IncludedItems>();
    let workspace_hirs = workspace_modules
        .iter()
        .map(|module| (module.name.as_ref(), module.hir.module()))
        .collect::<Vec<_>>();
    let core = if workspace_hirs.is_empty() {
        lower_runtime_module_with_items(entry_hir.module(), &included_items)
    } else {
        lower_runtime_module_with_workspace(entry_hir.module(), &workspace_hirs, &included_items)
    }
    .map_err(BackendUnitError::CoreLowering)?;
    validate_core_module(&core).map_err(BackendUnitError::CoreValidation)?;

    let lambda = lower_lambda_module(&core).map_err(BackendUnitError::LambdaLowering)?;
    validate_lambda_module(&lambda).map_err(BackendUnitError::LambdaValidation)?;

    let backend = lower_backend_module(&lambda, entry_hir.module())
        .map_err(BackendUnitError::BackendLowering)?;
    validate_program(&backend).map_err(BackendUnitError::BackendValidation)?;

    let fingerprint = WholeProgramFingerprint(stable_fingerprint(&backend));
    Ok(Arc::new(WholeProgramBackendUnit {
        entry: file,
        entry_hir,
        workspace_modules,
        included_items: normalized_included_items,
        core: Arc::new(core),
        lambda: Arc::new(lambda),
        backend: Arc::new(backend),
        fingerprint,
    }))
}

fn lower_runtime_fragment_backend_unit(
    file: SourceFile,
    entry_hir: Arc<HirModuleResult>,
    workspace_modules: Arc<[WorkspaceHirModule]>,
    fragment: &RuntimeFragmentSpec,
) -> Result<Arc<RuntimeFragmentBackendUnit>, BackendUnitError> {
    let workspace_hirs = workspace_modules
        .iter()
        .map(|module| (module.name.as_ref(), module.hir.module()))
        .collect::<Vec<_>>();
    let core = if workspace_hirs.is_empty() {
        lower_runtime_fragment(entry_hir.module(), fragment)
    } else {
        lower_runtime_fragment_with_workspace(entry_hir.module(), &workspace_hirs, fragment)
    }
    .map_err(BackendUnitError::CoreLowering)?;
    validate_core_module(&core.module).map_err(BackendUnitError::CoreValidation)?;

    let lambda = lower_lambda_module(&core.module).map_err(BackendUnitError::LambdaLowering)?;
    validate_lambda_module(&lambda).map_err(BackendUnitError::LambdaValidation)?;

    let backend = lower_backend_module(&lambda, entry_hir.module())
        .map_err(BackendUnitError::BackendLowering)?;
    validate_program(&backend).map_err(BackendUnitError::BackendValidation)?;

    let fingerprint = RuntimeFragmentFingerprint(stable_fingerprint(&backend));
    Ok(Arc::new(RuntimeFragmentBackendUnit {
        entry: file,
        entry_hir,
        fragment: Arc::new(fragment.clone()),
        core: Arc::new(core),
        lambda: Arc::new(lambda),
        backend: Arc::new(backend),
        fingerprint,
    }))
}

fn collect_workspace_hir_modules(
    db: &RootDatabase,
    file: SourceFile,
    entry_hir: &Arc<HirModuleResult>,
) -> Arc<[WorkspaceHirModule]> {
    let workspace = Workspace::discover(db, file);
    let mut module_map = HashMap::<String, WorkspaceHirModule>::new();
    for candidate in db.files() {
        if candidate == file {
            continue;
        }
        let Some(module_name) = workspace.module_name_for_file(db, candidate) else {
            continue;
        };
        if module_name.starts_with("aivi.") {
            continue;
        }
        module_map.insert(
            module_name.clone(),
            WorkspaceHirModule {
                name: module_name.into_boxed_str(),
                file: candidate,
                hir: hir_module(db, candidate),
            },
        );
    }

    let mut reachable = HashSet::<String>::new();
    let mut pending = VecDeque::<String>::new();
    enqueue_workspace_imports(entry_hir.module(), &module_map, &reachable, &mut pending);
    while let Some(name) = pending.pop_front() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        let Some(module) = module_map.get(&name) else {
            continue;
        };
        enqueue_workspace_imports(module.hir.module(), &module_map, &reachable, &mut pending);
    }

    if reachable.is_empty() {
        return Arc::from(Vec::<WorkspaceHirModule>::new());
    }

    let mut in_degree = HashMap::<String, usize>::new();
    let mut adjacency = HashMap::<String, Vec<String>>::new();
    for name in &reachable {
        let Some(module) = module_map.get(name) else {
            continue;
        };
        let mut deps = BTreeSet::<String>::new();
        for (_, import) in module.hir.module().imports().iter() {
            let Some(dep_name) = import.source_module.as_deref() else {
                continue;
            };
            if reachable.contains(dep_name) && dep_name != name {
                deps.insert(dep_name.to_owned());
            }
        }
        in_degree.insert(name.clone(), deps.len());
        for dep in deps {
            adjacency.entry(dep).or_default().push(name.clone());
        }
    }
    for dependents in adjacency.values_mut() {
        dependents.sort();
        dependents.dedup();
    }

    let mut ready = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    let mut ordered_names = Vec::with_capacity(reachable.len());
    while let Some(name) = ready.pop_first() {
        ordered_names.push(name.clone());
        if let Some(dependents) = adjacency.get(&name) {
            for dependent in dependents {
                let degree = in_degree
                    .get_mut(dependent)
                    .expect("reachable dependent must have an in-degree entry");
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    ready.insert(dependent.clone());
                }
            }
        }
    }

    if ordered_names.len() < reachable.len() {
        let processed = ordered_names.iter().cloned().collect::<HashSet<_>>();
        let mut remaining = reachable
            .into_iter()
            .filter(|name| !processed.contains(name))
            .collect::<Vec<_>>();
        remaining.sort();
        ordered_names.extend(remaining);
    }

    Arc::from(
        ordered_names
            .into_iter()
            .filter_map(|name| module_map.remove(&name))
            .collect::<Vec<_>>(),
    )
}

fn enqueue_workspace_imports(
    module: &aivi_hir::Module,
    module_map: &HashMap<String, WorkspaceHirModule>,
    reachable: &HashSet<String>,
    pending: &mut VecDeque<String>,
) {
    for (_, import) in module.imports().iter() {
        let Some(dep_name) = import.source_module.as_deref() else {
            continue;
        };
        if module_map.contains_key(dep_name) && !reachable.contains(dep_name) {
            pending.push_back(dep_name.to_owned());
        }
    }
}

fn normalize_included_items(included_items: &IncludedItems) -> Arc<[HirItemId]> {
    let mut items = included_items.iter().copied().collect::<Vec<_>>();
    items.sort_by_key(|item| item.as_raw());
    Arc::from(items)
}

fn whole_program_cache_key(
    file: SourceFile,
    normalized_included_items: &[HirItemId],
) -> WholeProgramUnitCacheKey {
    let mut hasher = FxHasher::default();
    for item in normalized_included_items {
        item.as_raw().hash(&mut hasher);
    }
    WholeProgramUnitCacheKey {
        file_id: file.id,
        included_items_key: hasher.finish(),
    }
}

fn runtime_fragment_cache_key(
    file: SourceFile,
    fragment: &RuntimeFragmentSpec,
) -> RuntimeFragmentUnitCacheKey {
    let mut hasher = FxHasher::default();
    format!("{fragment:?}").hash(&mut hasher);
    RuntimeFragmentUnitCacheKey {
        file_id: file.id,
        fragment_key: hasher.finish(),
    }
}

fn stable_fingerprint(program: &backend::Program) -> StableFingerprint {
    StableFingerprint::new(compute_program_fingerprint(program))
}

fn workspace_modules_match(left: &[WorkspaceHirModule], right: &[WorkspaceHirModule]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(lhs, rhs)| {
            lhs.name == rhs.name && lhs.file == rhs.file && Arc::ptr_eq(&lhs.hir, &rhs.hir)
        })
}

fn clone_cached_value<T, E: Clone>(value: &Arc<Result<Arc<T>, E>>) -> Result<Arc<T>, E> {
    match value.as_ref() {
        Ok(unit) => Ok(Arc::clone(unit)),
        Err(error) => Err(error.clone()),
    }
}
