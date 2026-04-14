pub struct LoweringResult<S = Resolved> {
    module: Module<S>,
    diagnostics: Vec<Diagnostic>,
}

impl<S> LoweringResult<S> {
    pub fn new(module: Module<S>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            module,
            diagnostics,
        }
    }

    pub fn module(&self) -> &Module<S> {
        &self.module
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn into_parts(self) -> (Module<S>, Vec<Diagnostic>) {
        (self.module, self.diagnostics)
    }
}

/// Lowers a syntax module to HIR, leaving all name references as
/// [`ResolutionState::Unresolved`]. Import bindings from `use` declarations are
/// resolved via `resolver` (needed to detect cycles and populate metadata for
/// imported bindings), but term/type/export references inside item bodies are
/// not resolved. Call [`resolve_imports`] on the result to fill those in.
pub fn lower_structure(
    module: &syn::Module,
    resolver: Option<&dyn crate::resolver::ImportResolver>,
) -> LoweringResult<Unresolved> {
    let null_resolver = crate::resolver::NullImportResolver;
    let mut lowerer = Lowerer::new(module.file, resolver.unwrap_or(&null_resolver));
    for item in &module.items {
        lowerer.lower_item(item);
    }
    lowerer.lower_ambient_prelude();
    LoweringResult::new(lowerer.module.into_unresolved(), lowerer.diagnostics)
}

/// Resolves all [`ResolutionState::Unresolved`] name references in a
/// structurally-lowered HIR module produced by [`lower_structure`].
///
/// This pass builds the module-level name namespaces, resolves every term,
/// type, and export reference, and validates cluster normalisation. It does
/// not call any external import resolver — import-binding resolution is
/// already complete after [`lower_structure`].
pub fn resolve_imports(module: Module<Unresolved>) -> LoweringResult {
    let null_resolver = crate::resolver::NullImportResolver;
    // The module is about to be resolved by this function; convert to the
    // resolved type so the Lowerer can work with it uniformly, then perform
    // the resolution pass which fills every Unresolved reference in place.
    let mut lowerer = Lowerer::from_module(module.mark_resolved(), &null_resolver);
    let namespaces = lowerer.build_namespaces();
    lowerer.resolve_module(&namespaces);
    lowerer.hoist_lambdas();
    lowerer.validate_cluster_normalization();
    LoweringResult::new(lowerer.module, lowerer.diagnostics)
}

pub fn lower_module(module: &syn::Module) -> LoweringResult {
    lower_module_with_resolver(module, None)
}

pub fn lower_module_with_resolver(
    module: &syn::Module,
    resolver: Option<&dyn crate::resolver::ImportResolver>,
) -> LoweringResult {
    let null_resolver = crate::resolver::NullImportResolver;
    let mut lowerer = Lowerer::new(module.file, resolver.unwrap_or(&null_resolver));
    for item in &module.items {
        lowerer.lower_item(item);
    }
    lowerer.lower_ambient_prelude();
    let namespaces = lowerer.build_namespaces();
    lowerer.resolve_module(&namespaces);
    lowerer.hoist_lambdas();
    lowerer.normalize_function_signature_annotations();
    lowerer.validate_cluster_normalization();
    crate::capability_handle_elaboration::elaborate_capability_handles(
        &mut lowerer.module,
        &mut lowerer.diagnostics,
    );
    crate::signal_metadata_elaboration::populate_signal_metadata(&mut lowerer.module);
    crate::resource_signal_elaboration::elaborate_resource_signal_companions(&mut lowerer.module);
    crate::signal_metadata_elaboration::populate_signal_metadata(&mut lowerer.module);
    LoweringResult::new(lowerer.module, lowerer.diagnostics)
}
