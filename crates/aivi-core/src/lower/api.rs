#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoweringErrors {
    errors: Vec<LoweringError>,
}

impl LoweringErrors {
    pub fn new(errors: Vec<LoweringError>) -> Self {
        Self { errors }
    }

    pub fn errors(&self) -> &[LoweringError] {
        &self.errors
    }

    pub fn into_errors(self) -> Vec<LoweringError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl std::fmt::Display for LoweringErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoweringErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoweringError {
    UnknownOwner {
        owner: HirItemId,
    },
    UnknownImport {
        import: ImportId,
    },
    BlockedGateStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        stage_index: usize,
        span: SourceSpan,
        blocked: BlockedGateStage,
    },
    BlockedTruthyFalsyStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        truthy_stage_index: usize,
        falsy_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedTruthyFalsyStage,
    },
    BlockedFanoutStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        map_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedFanoutSegment,
    },
    BlockedTemporalStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        stage_index: usize,
        span: SourceSpan,
        blocked: BlockedTemporalStage,
    },
    BlockedRecurrence {
        owner: HirItemId,
        pipe_expr: HirExprId,
        start_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedRecurrenceNode,
    },
    BlockedSourceLifecycle {
        owner: HirItemId,
        span: SourceSpan,
        blocked: BlockedSourceLifecycleNode,
    },
    BlockedDecodeProgram {
        owner: HirItemId,
        span: SourceSpan,
        blocked: BlockedSourceDecodeProgram,
    },
    BlockedGeneralExpr {
        owner: HirItemId,
        body_expr: HirExprId,
        span: SourceSpan,
        blocked: BlockedGeneralExprBody,
    },
    MissingGeneralExprElaboration {
        owner: HirItemId,
        span: SourceSpan,
    },
    MissingInstanceMemberElaboration {
        instance: HirItemId,
        member_index: usize,
        span: SourceSpan,
    },
    MissingDomainMemberElaboration {
        domain: HirItemId,
        member_index: usize,
        span: SourceSpan,
    },
    DuplicatePipeStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        stage_index: usize,
    },
    DuplicatePipeRecurrence {
        owner: HirItemId,
        pipe_expr: HirExprId,
    },
    DuplicateSourceOwner {
        owner: HirItemId,
    },
    DuplicateDecodeOwner {
        owner: HirItemId,
    },
    MissingSourceForDecode {
        owner: HirItemId,
    },
    DependencyOutsideCore {
        owner: HirItemId,
        dependency: HirItemId,
    },
    ArenaOverflow {
        arena: &'static str,
        attempted_len: usize,
    },
    UnsupportedClassMemberDispatch {
        owner: HirItemId,
        span: SourceSpan,
        class_name: Box<str>,
        member_name: Box<str>,
        subject: Box<str>,
        reason: &'static str,
    },
    UnsupportedImportBinding {
        import: ImportId,
        span: SourceSpan,
        name: Box<str>,
        reason: &'static str,
    },
    Validation(ValidationError),
    /// An internal structural invariant was violated during lowering. This indicates a bug in the
    /// compiler, not in user input, but is reported as an error rather than a panic so that the
    /// compiler can continue and surface any additional diagnostics.
    InternalInvariantViolated {
        message: &'static str,
    },
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOwner { owner } => {
                write!(f, "typed-core lowering cannot find owner item {owner}")
            }
            Self::UnknownImport { import } => {
                write!(f, "typed-core lowering cannot find import binding {import}")
            }
            Self::BlockedGateStage {
                owner,
                stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on gate stage {stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedTruthyFalsyStage {
                owner,
                truthy_stage_index,
                falsy_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on truthy/falsy pair {truthy_stage_index}/{falsy_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedFanoutStage {
                owner,
                map_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on fanout stage {map_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedTemporalStage {
                owner,
                stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on temporal stage {stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedRecurrence {
                owner,
                start_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on recurrence stage {start_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedSourceLifecycle { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on source lifecycle for item {owner}: {blocked:?}"
            ),
            Self::BlockedDecodeProgram { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on decode program for item {owner}: {blocked:?}"
            ),
            Self::BlockedGeneralExpr { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on general expression body for item {owner}: {blocked}"
            ),
            Self::MissingGeneralExprElaboration { owner, .. } => write!(
                f,
                "typed-core lowering could not find a complete general-expression elaboration for item {owner}"
            ),
            Self::MissingInstanceMemberElaboration {
                instance,
                member_index,
                ..
            } => write!(
                f,
                "typed-core lowering could not find a complete general-expression elaboration for instance item {instance} member {member_index}"
            ),
            Self::MissingDomainMemberElaboration {
                domain,
                member_index,
                ..
            } => write!(
                f,
                "typed-core lowering could not find a complete general-expression elaboration for domain item {domain} member {member_index}"
            ),
            Self::DuplicatePipeStage {
                owner,
                pipe_expr,
                stage_index,
            } => write!(
                f,
                "typed-core lowering saw duplicate stage {stage_index} for pipe {pipe_expr} owned by item {owner}"
            ),
            Self::DuplicatePipeRecurrence { owner, pipe_expr } => write!(
                f,
                "typed-core lowering saw duplicate recurrence attachment for pipe {pipe_expr} owned by item {owner}"
            ),
            Self::DuplicateSourceOwner { owner } => {
                write!(
                    f,
                    "typed-core lowering saw more than one source for item {owner}"
                )
            }
            Self::DuplicateDecodeOwner { owner } => {
                write!(
                    f,
                    "typed-core lowering saw more than one decode program for item {owner}"
                )
            }
            Self::MissingSourceForDecode { owner } => write!(
                f,
                "typed-core lowering cannot attach decode program because item {owner} has no lowered source node"
            ),
            Self::DependencyOutsideCore { owner, dependency } => write!(
                f,
                "typed-core lowering cannot map dependency {dependency} owned by item {owner} into the current core slice"
            ),
            Self::ArenaOverflow {
                arena,
                attempted_len,
            } => write!(
                f,
                "typed-core {arena} arena overflowed after {attempted_len} entries"
            ),
            Self::UnsupportedClassMemberDispatch {
                class_name,
                member_name,
                subject,
                reason,
                ..
            } => write!(
                f,
                "typed-core lowering cannot lower overloaded class member `{class_name}.{member_name}` for `{subject}`: {reason}"
            ),
            Self::UnsupportedImportBinding {
                import,
                name,
                reason,
                ..
            } => write!(
                f,
                "typed-core lowering cannot synthesize imported binding `{name}` ({import}): {reason}"
            ),
            Self::Validation(error) => write!(f, "typed-core validation failed: {error}"),
            Self::InternalInvariantViolated { message } => {
                write!(
                    f,
                    "typed-core lowering internal invariant violated: {message}"
                )
            }
        }
    }
}

/// Lower a fully elaborated HIR module into a typed-core module.
pub fn lower_module(hir: &aivi_hir::Module) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new(hir).build()
}

pub fn lower_module_with_items(
    hir: &aivi_hir::Module,
    included_items: &HashSet<HirItemId>,
) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new_with_items(hir, included_items).build()
}

pub fn lower_runtime_module(hir: &aivi_hir::Module) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new_runtime(hir).build()
}

pub fn lower_runtime_module_with_items(
    hir: &aivi_hir::Module,
    included_items: &HashSet<HirItemId>,
) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new_runtime_with_items(hir, included_items).build()
}

/// Like `lower_runtime_module_with_items` but also compiles workspace module HIRs
/// so that their functions are available as pre-compiled items (with real bodies)
/// when entry-module imports are resolved.
///
/// `workspace_hirs` must be ordered so that each module appears before any module
/// that depends on it (topological dependency order).
pub fn lower_runtime_module_with_workspace<'a>(
    hir: &'a aivi_hir::Module,
    workspace_hirs: &[(&str, &'a aivi_hir::Module)],
    included_items: &HashSet<HirItemId>,
) -> Result<Module, LoweringErrors> {
    let included_items = included_items
        .iter()
        .copied()
        .filter(|item_id| !is_markup_value(hir, *item_id))
        .collect::<HashSet<_>>();
    let mut lowerer = ModuleLowerer::new_internal(hir, Some(included_items));
    // Workspace module origins must not overlap with the entry module's origin range:
    //   [0, hir_item_count)                 — entry module real item origins
    //   [hir_item_count, next_synthetic)     — entry module signal import stub origins
    //   [next_synthetic, ...)               — entry module synthetic item origins
    // By starting workspace origins at next_synthetic_item_origin_raw, each workspace
    // module receives a non-overlapping origin slice above the entry module's reserved range.
    // After all workspace modules are compiled, we advance next_synthetic_item_origin_raw
    // to ws_origin_base so that the entry module's own synthetic items (domain members, pipes,
    // etc.) start above all workspace module items and don't collide with them.
    lowerer.ws_origin_base = lowerer.next_synthetic_item_origin_raw;
    for (name, ws_hir) in workspace_hirs {
        lowerer.compile_workspace_module(name, ws_hir)?;
    }
    // Advance the entry module's synthetic item counter past all workspace-allocated origins.
    lowerer.next_synthetic_item_origin_raw = lowerer.ws_origin_base;
    lowerer.build()
}

fn validate_general_expr_report_completeness(
    hir: &aivi_hir::Module,
    report: &aivi_hir::GeneralExprElaborationReport,
    includes_item: impl Fn(HirItemId) -> bool,
) -> Vec<LoweringError> {
    let item_owners = report
        .items()
        .iter()
        .map(|item| item.owner)
        .collect::<HashSet<_>>();
    let domain_members = report
        .domain_members()
        .iter()
        .map(|item| (item.domain_owner, item.member_index))
        .collect::<HashSet<_>>();
    let instance_members = report
        .instance_members()
        .iter()
        .map(|item| (item.instance_owner, item.member_index))
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();
    for (item_id, item) in hir.items().iter() {
        if hir.ambient_items().contains(&item_id) || !includes_item(item_id) {
            continue;
        }
        match item {
            HirItem::Value(value) if !item_owners.contains(&item_id) => {
                errors.push(LoweringError::MissingGeneralExprElaboration {
                    owner: item_id,
                    span: value.header.span,
                });
            }
            HirItem::Function(function) if !item_owners.contains(&item_id) => {
                errors.push(LoweringError::MissingGeneralExprElaboration {
                    owner: item_id,
                    span: function.header.span,
                });
            }
            HirItem::Signal(signal) if signal.body.is_some() && !item_owners.contains(&item_id) => {
                errors.push(LoweringError::MissingGeneralExprElaboration {
                    owner: item_id,
                    span: signal.header.span,
                });
            }
            HirItem::Domain(domain) => {
                for (member_index, member) in domain.members.iter().enumerate() {
                    if member.body.is_some() && !domain_members.contains(&(item_id, member_index)) {
                        errors.push(LoweringError::MissingDomainMemberElaboration {
                            domain: item_id,
                            member_index,
                            span: member.span,
                        });
                    }
                }
            }
            HirItem::Instance(instance) => {
                for (member_index, member) in instance.members.iter().enumerate() {
                    if !instance_members.contains(&(item_id, member_index)) {
                        errors.push(LoweringError::MissingInstanceMemberElaboration {
                            instance: item_id,
                            member_index,
                            span: member.span,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    errors
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFragmentSpec {
    pub name: Box<str>,
    pub owner: HirItemId,
    pub body_expr: HirExprId,
    pub parameters: Vec<GeneralExprParameter>,
    pub body: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoweredRuntimeFragment {
    pub entry_name: Box<str>,
    pub module: Module,
}

pub fn lower_runtime_fragment(
    hir: &aivi_hir::Module,
    fragment: &RuntimeFragmentSpec,
) -> Result<LoweredRuntimeFragment, LoweringErrors> {
    RuntimeFragmentLowerer::new(hir, fragment).build()
}

pub fn runtime_fragment_included_items(
    hir: &aivi_hir::Module,
    fragment: &RuntimeFragmentSpec,
) -> HashSet<HirItemId> {
    RuntimeFragmentItemCollector::new(hir, fragment).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PipeKey {
    owner: HirItemId,
    pipe_expr: HirExprId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct InstanceMemberKey {
    instance: HirItemId,
    member_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct DomainMemberKey {
    domain: HirItemId,
    member_index: usize,
}

struct PipeBuilder {
    owner: ItemId,
    origin: PipeOrigin,
    stages: BTreeMap<usize, PendingStage>,
    recurrence: Option<PipeRecurrence>,
}

enum PendingStage {
    Lowered {
        span: SourceSpan,
        input_subject: Type,
        result_subject: Type,
        kind: StageKind,
    },
}
