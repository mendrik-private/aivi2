/// Validation strictness for HIR modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Structural,
    RequireResolvedNames,
}

/// Aggregated HIR validation result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationReport {
    diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn extend(&mut self, other: ValidationReport) {
        self.diagnostics.extend(other.diagnostics);
    }
}

/// Validates structural integrity: roots, imports, decorators, types, patterns,
/// expressions, markup/control nodes, clusters, and items.
pub fn validate_structure(module: &Module, mode: ValidationMode) -> ValidationReport {
    let mut v = Validator {
        module,
        mode,
        diagnostics: Vec::new(),
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
    };
    v.validate_roots();
    v.validate_type_parameters();
    v.validate_imports();
    v.validate_decorators();
    v.validate_types();
    v.validate_patterns();
    v.validate_exprs();
    v.validate_markup_nodes();
    v.validate_control_nodes();
    v.validate_clusters();
    v.validate_items();
    ValidationReport::new(v.diagnostics)
}

/// Validates binding uniqueness and signal cycle freedom.
pub fn validate_bindings(module: &Module, mode: ValidationMode) -> ValidationReport {
    let mut v = Validator {
        module,
        mode,
        diagnostics: Vec::new(),
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
    };
    v.validate_bindings();
    v.validate_signal_cycles();
    ValidationReport::new(v.diagnostics)
}

/// Validates the type system: kinds, instances, source contracts, expression
/// types, constructor arity, and pipe semantics.
pub fn validate_types(module: &Module, mode: ValidationMode) -> ValidationReport {
    let mut v = Validator {
        module,
        mode,
        diagnostics: Vec::new(),
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
    };
    v.validate_type_kinds();
    v.validate_instance_items();
    v.validate_source_contract_types();
    v.validate_expression_types();
    v.validate_constructor_arity();
    v.validate_pipe_semantics();
    ValidationReport::new(v.diagnostics)
}

pub fn validate_module(module: &Module, mode: ValidationMode) -> ValidationReport {
    let mut report = validate_structure(module, mode);
    report.extend(validate_bindings(module, mode));
    report.extend(validate_types(module, mode));
    let mut v = Validator {
        module,
        mode,
        diagnostics: Vec::new(),
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
    };
    v.validate_decorator_semantics();
    report.extend(ValidationReport::new(v.diagnostics));
    report
}

pub(crate) struct Validator<'a> {
    pub(crate) module: &'a Module,
    pub(crate) mode: ValidationMode,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) kind_item_cache: HashMap<ItemId, Option<Kind>>,
    pub(crate) kind_item_stack: HashSet<ItemId>,
}

const REGEX_LITERAL_PREFIX_LEN: usize = 3;
const REGEX_NEST_LIMIT: u32 = 256;
