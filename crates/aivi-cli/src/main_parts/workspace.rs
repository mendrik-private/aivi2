struct WorkspaceFrontend {
    db: RootDatabase,
    entry: QuerySourceFile,
}

#[derive(Clone, Copy)]
struct BackendQueryContext<'a> {
    db: &'a RootDatabase,
    entry: QuerySourceFile,
}

impl WorkspaceFrontend {
    fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let db = RootDatabase::new();
        let entry = QuerySourceFile::new(&db, path.to_path_buf(), text);
        Ok(Self { db, entry })
    }

    fn warm(&self) {
        let _ = query_hir_module(&self.db, self.entry);
    }

    fn files(&self) -> Vec<QuerySourceFile> {
        self.db.files()
    }

    fn sources(&self) -> SourceDatabase {
        self.db.source_database()
    }
}

struct WorkspaceHirSnapshot {
    frontend: WorkspaceFrontend,
    sources: SourceDatabase,
    files: Vec<QuerySourceFile>,
}

impl WorkspaceHirSnapshot {
    fn load(path: &Path) -> Result<Self, String> {
        let frontend = WorkspaceFrontend::load(path)?;
        frontend.warm();
        let sources = frontend.sources();
        let files = frontend.files();
        Ok(Self {
            frontend,
            sources,
            files,
        })
    }

    fn entry_parsed(&self) -> Arc<aivi_query::ParsedFileResult> {
        query_parsed_file(&self.frontend.db, self.frontend.entry)
    }

    fn entry_hir(&self) -> Arc<aivi_query::HirModuleResult> {
        query_hir_module(&self.frontend.db, self.frontend.entry)
    }

    fn backend_query_context(&self) -> BackendQueryContext<'_> {
        BackendQueryContext {
            db: &self.frontend.db,
            entry: self.frontend.entry,
        }
    }
}

/// Compute a module name from a file path relative to the workspace root.
/// Returns e.g. "libs.types" for "<root>/libs/types.aivi".
fn module_name_from_path(workspace_root: &Path, file_path: &Path) -> Option<String> {
    let relative = file_path.strip_prefix(workspace_root).ok()?;
    if relative.extension()?.to_str()? != "aivi" {
        return None;
    }
    let mut segments = relative
        .iter()
        .map(|seg| seg.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    let file_name = segments.pop()?;
    let stem = Path::new(&file_name).file_stem()?.to_str()?.to_owned();
    segments.push(stem);
    Some(segments.join("."))
}

/// Collect all non-entry workspace HIR modules in topological dependency order
/// (dependencies before dependents) so that workspace function bodies are
/// available when later modules reference them.
fn collect_workspace_hirs_sorted(
    snapshot: &WorkspaceHirSnapshot,
) -> Vec<(String, Arc<HirModuleResult>)> {
    let entry_path_raw = snapshot.frontend.entry.path(&snapshot.frontend.db);
    // Canonicalize to absolute paths so strip_prefix works correctly when aivi is
    // invoked with a relative path (e.g. `aivi run apps/ui/main.aivi`).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry_path = std::fs::canonicalize(&entry_path_raw)
        .or_else(|_| std::fs::canonicalize(cwd.join(&entry_path_raw)))
        .unwrap_or_else(|_| cwd.join(&entry_path_raw));
    let workspace_root_raw = discover_workspace_root(&entry_path);
    let workspace_root = std::fs::canonicalize(&workspace_root_raw).unwrap_or(workspace_root_raw);

    // Collect (module_name, file, hir) for all non-entry, non-stdlib workspace files.
    // Stdlib modules (aivi.*) are excluded because their intrinsic functions can't be
    // synthesized through the workspace compilation path; they are handled via
    // hoisted item bodies or synthesize_import_body in the entry module's core lowering.
    let mut ws_modules: Vec<(String, QuerySourceFile, Arc<HirModuleResult>)> = Vec::new();
    for &file in &snapshot.files {
        let path_raw = file.path(&snapshot.frontend.db);
        let path = std::fs::canonicalize(&path_raw)
            .or_else(|_| std::fs::canonicalize(cwd.join(&path_raw)))
            .unwrap_or_else(|_| cwd.join(&path_raw));
        if path == entry_path {
            continue;
        }
        let Some(module_name) = module_name_from_path(&workspace_root, &path) else {
            continue;
        };
        // Skip bundled stdlib modules (e.g. aivi.list, aivi.option, aivi.matrix).
        // Their intrinsic/native functions cannot be compiled via workspace_name_maps.
        if module_name.starts_with("aivi.") {
            continue;
        }
        let hir = query_hir_module(&snapshot.frontend.db, file);
        ws_modules.push((module_name, file, hir));
    }

    // Restrict to the entry module's reachable workspace dependency closure. Including
    // unrelated apps in the same workspace pollutes runtime lowering with extra items
    // and can cause synthetic-origin collisions across independent app graphs.
    let ws_names: std::collections::HashSet<&str> =
        ws_modules.iter().map(|(n, _, _)| n.as_str()).collect();
    let module_hirs: HashMap<String, Arc<HirModuleResult>> = ws_modules
        .iter()
        .map(|(name, _, hir)| (name.clone(), hir.clone()))
        .collect();
    let mut reachable = HashSet::<String>::new();
    let mut queue = VecDeque::<String>::new();
    for (_, import) in snapshot.entry_hir().module().imports().iter() {
        let Some(dep_name) = import.source_module.as_deref() else {
            continue;
        };
        if ws_names.contains(dep_name) {
            queue.push_back(dep_name.to_owned());
        }
    }
    while let Some(name) = queue.pop_front() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        let Some(hir) = module_hirs.get(&name) else {
            continue;
        };
        for (_, import) in hir.module().imports().iter() {
            let Some(dep_name) = import.source_module.as_deref() else {
                continue;
            };
            if ws_names.contains(dep_name) && !reachable.contains(dep_name) {
                queue.push_back(dep_name.to_owned());
            }
        }
    }

    // Build dependency graph: module_name → set of reachable workspace module names it depends on.
    let deps: Vec<(String, Vec<String>)> = ws_modules
        .iter()
        .filter(|(name, _, _)| reachable.contains(name))
        .map(|(name, _, hir)| {
            let module_hir = hir.module();
            let mut module_deps = Vec::new();
            for (_, import) in module_hir.imports().iter() {
                let Some(dep_name) = import.source_module.as_deref() else {
                    continue;
                };
                if reachable.contains(dep_name) && dep_name != name {
                    module_deps.push(dep_name.to_owned());
                }
            }
            module_deps.sort();
            module_deps.dedup();
            (name.clone(), module_deps)
        })
        .collect();

    // Topological sort (Kahn's algorithm):
    // in_degree[A] = number of A's unprocessed dependencies.
    let mut in_degree: HashMap<String, usize> =
        deps.iter().map(|(n, d)| (n.clone(), d.len())).collect();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (name, module_deps) in &deps {
        for dep in module_deps {
            adjacency.entry(dep.clone()).or_default().push(name.clone());
        }
    }
    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    let mut sorted_names: Vec<String> = Vec::new();
    while let Some(name) = queue.pop_front() {
        sorted_names.push(name.clone());
        for dependent in adjacency.get(&name).cloned().unwrap_or_default() {
            let count = in_degree.entry(dependent.clone()).or_insert(0);
            *count = count.saturating_sub(1);
            if *count == 0 {
                queue.push_back(dependent);
            }
        }
    }

    // Build final result in topological order.
    let module_map: HashMap<String, Arc<HirModuleResult>> = ws_modules
        .into_iter()
        .filter(|(name, _, _)| reachable.contains(name))
        .map(|(name, _, hir)| (name, hir))
        .collect();
    sorted_names
        .into_iter()
        .filter_map(|name| {
            let hir = module_map.get(&name)?.clone();
            Some((name, hir))
        })
        .collect()
}

fn workspace_syntax_failed(
    snapshot: &WorkspaceHirSnapshot,
    mut print: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
) -> bool {
    let mut failed = false;
    for file in &snapshot.files {
        let parsed = query_parsed_file(&snapshot.frontend.db, *file);
        failed |= print(&snapshot.sources, parsed.diagnostics());
    }
    failed
}

fn workspace_hir_failed(
    snapshot: &WorkspaceHirSnapshot,
    mut print_hir: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
    mut print_validation: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
) -> (bool, bool) {
    let mut lowering_failed = false;
    let mut validation_failed = false;
    for file in &snapshot.files {
        let hir = query_hir_module(&snapshot.frontend.db, *file);
        let file_lowering_failed = print_hir(&snapshot.sources, hir.hir_diagnostics());
        lowering_failed |= file_lowering_failed;
        let validation_mode = if file_lowering_failed {
            ValidationMode::Structural
        } else {
            ValidationMode::RequireResolvedNames
        };
        let validation = hir.module().validate(validation_mode);
        validation_failed |= print_validation(&snapshot.sources, validation.diagnostics());
    }
    (lowering_failed, validation_failed)
}

