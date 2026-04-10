use aivi_base::SourceSpan;
use aivi_typing::{BuiltinSourceProvider, SourceCancellationPolicy, SourceOptionWakeupCause};

use crate::{
    BlockedGeneralExpr, DecoratorId, DecoratorPayload, ExprId, ExprKind, GateRuntimeExpr, Item,
    ItemId, Module, Name, ProjectionBase, ResolutionState, SignalItem, SourceDecorator,
    SourceMetadata, SourceProviderRef, TermResolution,
    general_expr_elaboration::elaborate_runtime_expr,
    signal_metadata_elaboration::collect_signal_dependencies_for_expr,
};

/// Focused source-lifecycle plans derived from resolved `@source` signals.
///
/// This report keeps lifecycle policy in the same staged HIR handoff family as gates, fanout, and
/// recurrence. It does not pretend a runtime exists already, but it does make the runtime-owned
/// invariants explicit:
/// - every source site has one stable instance identity,
/// - owner teardown disposes that instance,
/// - reactive reconfiguration replaces the superseded resource before replacement publication,
/// - stale publications from old work generations must be suppressed,
/// - and request-like built-ins additionally request best-effort in-flight cancellation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceLifecycleElaborationReport {
    nodes: Vec<SourceLifecycleNodeElaboration>,
}

impl SourceLifecycleElaborationReport {
    pub fn new(nodes: Vec<SourceLifecycleNodeElaboration>) -> Self {
        Self { nodes }
    }

    pub fn nodes(&self) -> &[SourceLifecycleNodeElaboration] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<SourceLifecycleNodeElaboration> {
        self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLifecycleNodeElaboration {
    pub owner: ItemId,
    pub source_instance: SourceInstanceId,
    pub source_span: SourceSpan,
    pub outcome: SourceLifecycleNodeOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceLifecycleNodeOutcome {
    Planned(SourceLifecyclePlan),
    Blocked(BlockedSourceLifecycleNode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceInstanceId {
    decorator: DecoratorId,
}

impl SourceInstanceId {
    pub const fn from_decorator(decorator: DecoratorId) -> Self {
        Self { decorator }
    }

    pub const fn decorator(self) -> DecoratorId {
        self.decorator
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOptionSignalBinding {
    pub option_span: SourceSpan,
    pub option_name: Name,
    pub expr: ExprId,
    pub signal: Option<ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceArgumentValueBinding {
    pub expr: ExprId,
    pub runtime_expr: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOptionValueBinding {
    pub option_span: SourceSpan,
    pub option_name: Name,
    pub expr: ExprId,
    pub runtime_expr: GateRuntimeExpr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceTeardownPolicy {
    DisposeOnOwnerTeardown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceReplacementPolicy {
    DisposeSupersededBeforePublish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceStaleWorkPolicy {
    DropStalePublications,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLifecyclePlan {
    pub instance: SourceInstanceId,
    pub provider: SourceProviderRef,
    pub teardown: SourceTeardownPolicy,
    pub replacement: SourceReplacementPolicy,
    pub arguments: Vec<SourceArgumentValueBinding>,
    pub options: Vec<SourceOptionValueBinding>,
    pub reconfiguration_dependencies: Vec<ItemId>,
    pub explicit_triggers: Vec<SourceOptionSignalBinding>,
    pub active_when: Option<SourceOptionSignalBinding>,
    pub cancellation: SourceCancellationPolicy,
    pub stale_work: SourceStaleWorkPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedSourceLifecycleNode {
    pub provider: SourceProviderRef,
    pub explicit_triggers: Vec<SourceOptionSignalBinding>,
    pub active_when: Option<SourceOptionSignalBinding>,
    pub reconfiguration_dependencies: Vec<ItemId>,
    pub blockers: Vec<SourceLifecycleElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceLifecycleElaborationBlocker {
    MissingSourceMetadata,
    MissingProvider,
    InvalidProviderShape {
        key: Box<str>,
    },
    UnsupportedArgumentRuntimeExpr {
        index: usize,
        blocked: BlockedGeneralExpr,
    },
    UnsupportedOptionRuntimeExpr {
        option_name: Name,
        blocked: BlockedGeneralExpr,
    },
}

pub fn elaborate_source_lifecycles(module: &Module) -> SourceLifecycleElaborationReport {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    let module = &module;
    let mut nodes = Vec::new();
    for (owner, item) in module.items().iter() {
        let Item::Signal(signal) = item else {
            continue;
        };
        if signal.is_source_capability_handle {
            continue;
        }
        let Some((decorator_id, source_span, source)) = signal_source_decorator(module, signal)
        else {
            continue;
        };
        nodes.push(SourceLifecycleNodeElaboration {
            owner,
            source_instance: SourceInstanceId::from_decorator(decorator_id),
            source_span,
            outcome: elaborate_source_lifecycle_signal(
                module,
                decorator_id,
                source,
                signal.source_metadata.as_ref(),
            ),
        });
    }
    SourceLifecycleElaborationReport::new(nodes)
}

fn elaborate_source_lifecycle_signal(
    module: &Module,
    decorator_id: DecoratorId,
    source: &SourceDecorator,
    metadata: Option<&SourceMetadata>,
) -> SourceLifecycleNodeOutcome {
    let provider = metadata
        .map(|metadata| metadata.provider.clone())
        .unwrap_or_else(|| SourceProviderRef::from_path(source.provider.as_ref()));
    let explicit_triggers = explicit_trigger_bindings(module, source, &provider);
    let active_when = active_when_binding(module, source, &provider);
    let mut blockers = Vec::new();
    let arguments = source_argument_value_bindings(module, source, &mut blockers);
    let options = source_option_value_bindings(module, source, &mut blockers);
    let mut reconfiguration_dependencies = Vec::new();

    match &provider {
        SourceProviderRef::Missing => {
            blockers.push(SourceLifecycleElaborationBlocker::MissingProvider)
        }
        SourceProviderRef::InvalidShape(key) => blockers
            .push(SourceLifecycleElaborationBlocker::InvalidProviderShape { key: key.clone() }),
        SourceProviderRef::Builtin(_) | SourceProviderRef::Custom(_) => {}
    }

    let Some(metadata) = metadata else {
        blockers.push(SourceLifecycleElaborationBlocker::MissingSourceMetadata);
        return SourceLifecycleNodeOutcome::Blocked(BlockedSourceLifecycleNode {
            provider,
            explicit_triggers,
            active_when,
            reconfiguration_dependencies,
            blockers,
        });
    };

    reconfiguration_dependencies = metadata.lifecycle_dependencies.reconfiguration.clone();
    let cancellation = lifecycle_cancellation_policy(&provider);

    if blockers.is_empty() {
        SourceLifecycleNodeOutcome::Planned(SourceLifecyclePlan {
            instance: SourceInstanceId::from_decorator(decorator_id),
            provider,
            teardown: SourceTeardownPolicy::DisposeOnOwnerTeardown,
            replacement: SourceReplacementPolicy::DisposeSupersededBeforePublish,
            arguments,
            options,
            reconfiguration_dependencies,
            explicit_triggers,
            active_when,
            cancellation,
            stale_work: SourceStaleWorkPolicy::DropStalePublications,
        })
    } else {
        SourceLifecycleNodeOutcome::Blocked(BlockedSourceLifecycleNode {
            provider,
            explicit_triggers,
            active_when,
            reconfiguration_dependencies,
            blockers,
        })
    }
}

fn lifecycle_cancellation_policy(provider: &SourceProviderRef) -> SourceCancellationPolicy {
    provider
        .builtin()
        .map(|provider| provider.contract().lifecycle().cancellation())
        .unwrap_or(SourceCancellationPolicy::ProviderManaged)
}

fn source_argument_value_bindings(
    module: &Module,
    source: &SourceDecorator,
    blockers: &mut Vec<SourceLifecycleElaborationBlocker>,
) -> Vec<SourceArgumentValueBinding> {
    let mut bindings = Vec::with_capacity(source.arguments.len());
    for (index, expr) in source.arguments.iter().copied().enumerate() {
        match elaborate_runtime_expr(module, expr, None) {
            Ok(runtime_expr) => bindings.push(SourceArgumentValueBinding { expr, runtime_expr }),
            Err(blocked) => {
                blockers.push(
                    SourceLifecycleElaborationBlocker::UnsupportedArgumentRuntimeExpr {
                        index,
                        blocked,
                    },
                );
            }
        }
    }
    bindings
}

fn source_option_value_bindings(
    module: &Module,
    source: &SourceDecorator,
    blockers: &mut Vec<SourceLifecycleElaborationBlocker>,
) -> Vec<SourceOptionValueBinding> {
    let Some(options) = source.options else {
        return Vec::new();
    };
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return Vec::new();
    };
    let mut bindings = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        match elaborate_runtime_expr(module, field.value, None) {
            Ok(runtime_expr) => bindings.push(SourceOptionValueBinding {
                option_span: field.span,
                option_name: field.label.clone(),
                expr: field.value,
                runtime_expr,
            }),
            Err(blocked) => blockers.push(
                SourceLifecycleElaborationBlocker::UnsupportedOptionRuntimeExpr {
                    option_name: field.label.clone(),
                    blocked,
                },
            ),
        }
    }
    bindings
}

fn explicit_trigger_bindings(
    module: &Module,
    source: &SourceDecorator,
    provider: &SourceProviderRef,
) -> Vec<SourceOptionSignalBinding> {
    let Some(builtin_provider) = provider.builtin() else {
        return Vec::new();
    };
    let Some(options) = source.options else {
        return Vec::new();
    };
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return Vec::new();
    };
    let contract = builtin_provider.contract();
    record
        .fields
        .iter()
        .filter(|field| {
            contract
                .wakeup_option(field.label.text())
                .map(|option| option.cause())
                == Some(SourceOptionWakeupCause::TriggerSignal)
        })
        .map(|field| SourceOptionSignalBinding {
            option_span: field.span,
            option_name: field.label.clone(),
            expr: field.value,
            signal: resolve_source_option_signal_binding(
                module,
                field.value,
                matches!(builtin_provider, BuiltinSourceProvider::DbLive)
                    && field.label.text() == "refreshOn",
            ),
        })
        .collect()
}

/// Extracts the `activeWhen` binding from a built-in source's options record for lifecycle
/// planning.
///
/// This function performs structural extraction only — it locates the `activeWhen` field and
/// records its `ExprId` for use by the lifecycle planner.  Type validation (enforcing that the
/// expression has type `Signal Bool`) is handled separately by
/// `validate_builtin_source_decorator_contract_types` in `validate.rs`, which checks every source
/// option field against the provider contract.  Programs with a type-mismatched `activeWhen` will
/// therefore carry a `source-option-type-mismatch` diagnostic and will never reach the runtime.
fn active_when_binding(
    module: &Module,
    source: &SourceDecorator,
    provider: &SourceProviderRef,
) -> Option<SourceOptionSignalBinding> {
    let builtin_provider = provider.builtin()?;
    let options = source.options?;
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return None;
    };
    builtin_provider.contract().option("activeWhen")?;
    let field = record
        .fields
        .iter()
        .find(|field| field.label.text() == "activeWhen")?;
    Some(SourceOptionSignalBinding {
        option_span: field.span,
        option_name: field.label.clone(),
        expr: field.value,
        signal: resolve_source_option_signal_binding(module, field.value, false),
    })
}

fn resolve_source_option_signal_binding(
    module: &Module,
    expr: ExprId,
    allow_db_changed_projection: bool,
) -> Option<ItemId> {
    resolve_direct_signal_binding(module, expr).or_else(|| {
        allow_db_changed_projection
            .then(|| resolve_db_changed_projection_signal(module, expr))
            .flatten()
    })
}

fn resolve_direct_signal_binding(module: &Module, expr: ExprId) -> Option<ItemId> {
    let ExprKind::Name(reference) = &module.exprs()[expr].kind else {
        return None;
    };
    let ResolutionState::Resolved(TermResolution::Item(item)) = reference.resolution else {
        return None;
    };
    matches!(&module.items()[item], Item::Signal(_)).then_some(item)
}

fn resolve_db_changed_projection_signal(module: &Module, expr: ExprId) -> Option<ItemId> {
    if !is_db_changed_projection(module, expr) {
        return None;
    }
    match collect_signal_dependencies_for_expr(module, expr).as_slice() {
        [signal] => Some(*signal),
        _ => None,
    }
}

fn is_db_changed_projection(module: &Module, expr: ExprId) -> bool {
    matches!(
        &module.exprs()[expr].kind,
        ExprKind::Projection {
            base: ProjectionBase::Expr(_),
            path,
        } if path.segments().len() == 1 && path.segments().first().text() == "changed"
    )
}

fn signal_source_decorator<'a>(
    module: &'a Module,
    item: &'a SignalItem,
) -> Option<(DecoratorId, SourceSpan, &'a SourceDecorator)> {
    item.header.decorators.iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some((*decorator_id, decorator.span, source)),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        SourceCancellationPolicy, SourceLifecycleElaborationBlocker, SourceLifecycleNodeOutcome,
        SourceReplacementPolicy, SourceStaleWorkPolicy, SourceTeardownPolicy,
        elaborate_source_lifecycles,
    };
    use crate::test_support::{item_name, lower_fixture, lower_text};
    use crate::{Item, SourceProviderRef};

    fn dependency_names(module: &crate::Module, dependencies: &[crate::ItemId]) -> Vec<String> {
        dependencies
            .iter()
            .map(|item_id| match &module.items()[*item_id] {
                Item::Signal(item) => item.name.text().to_owned(),
                other => panic!(
                    "expected source lifecycle dependency to point at a signal, found {other:?}"
                ),
            })
            .collect()
    }

    fn item_id(module: &crate::Module, name: &str) -> crate::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Type(item) if item.name.text() == name => Some(item_id),
                Item::Value(item) if item.name.text() == name => Some(item_id),
                Item::Function(item) if item.name.text() == name => Some(item_id),
                Item::Signal(item) if item.name.text() == name => Some(item_id),
                Item::Class(item) if item.name.text() == name => Some(item_id),
                Item::Domain(item) if item.name.text() == name => Some(item_id),
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    #[test]
    fn elaborates_request_like_source_lifecycle_plans() {
        let lowered = lower_text(
            "request_source_lifecycle.aivi",
            r#"
domain Duration over Int = {
    literal sec : Int -> Duration
}
signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval : Signal Duration = 5sec

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "request-like source lifecycle fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_lifecycles(lowered.module());
        let users = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "users")
            .expect("expected lifecycle plan for users");
        let Item::Signal(signal) = &lowered.module().items()[users.owner] else {
            panic!("expected users to stay a signal item");
        };

        match &users.outcome {
            SourceLifecycleNodeOutcome::Planned(plan) => {
                assert_eq!(plan.instance.decorator(), signal.header.decorators[0]);
                assert_eq!(
                    plan.provider,
                    SourceProviderRef::Builtin(aivi_typing::BuiltinSourceProvider::HttpGet)
                );
                assert_eq!(plan.teardown, SourceTeardownPolicy::DisposeOnOwnerTeardown);
                assert_eq!(
                    plan.replacement,
                    SourceReplacementPolicy::DisposeSupersededBeforePublish
                );
                assert_eq!(
                    dependency_names(lowered.module(), &plan.reconfiguration_dependencies),
                    vec!["apiHost".to_owned(), "pollInterval".to_owned()]
                );
                assert_eq!(plan.arguments.len(), 1);
                assert_eq!(plan.options.len(), 3);
                assert!(matches!(
                    plan.arguments[0].runtime_expr.kind,
                    crate::GateRuntimeExprKind::Text(_)
                ));
                assert_eq!(plan.options[0].option_name.text(), "refreshOn");
                assert_eq!(plan.options[1].option_name.text(), "activeWhen");
                assert_eq!(plan.options[2].option_name.text(), "refreshEvery");
                assert_eq!(plan.explicit_triggers.len(), 1);
                assert_eq!(plan.explicit_triggers[0].option_name.text(), "refreshOn");
                assert_eq!(
                    plan.explicit_triggers[0].signal,
                    Some(item_id(lowered.module(), "refresh"))
                );
                let active_when = plan
                    .active_when
                    .as_ref()
                    .expect("http source should preserve activeWhen binding");
                assert_eq!(active_when.option_name.text(), "activeWhen");
                assert_eq!(
                    active_when.signal,
                    Some(item_id(lowered.module(), "enabled"))
                );
                assert_eq!(plan.cancellation, SourceCancellationPolicy::CancelInFlight);
                assert_eq!(
                    plan.stale_work,
                    SourceStaleWorkPolicy::DropStalePublications
                );
            }
            other => panic!("expected planned request-like source lifecycle, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_db_live_refresh_on_changed_projection() {
        let lowered = lower_text(
            "db_live_changed_projection.aivi",
            r#"
type TableRef A = {
    changed: Signal Unit
}

signal usersChanged : Signal Unit

value users : TableRef Int = {
    changed: usersChanged
}

@source db.live with {
    refreshOn: users.changed
}
signal rows : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "db.live projection fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_lifecycles(lowered.module());
        let rows = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "rows")
            .expect("expected lifecycle plan for rows");

        match &rows.outcome {
            SourceLifecycleNodeOutcome::Planned(plan) => {
                assert_eq!(
                    plan.provider,
                    SourceProviderRef::Builtin(aivi_typing::BuiltinSourceProvider::DbLive)
                );
                assert_eq!(plan.explicit_triggers.len(), 1);
                assert_eq!(plan.explicit_triggers[0].option_name.text(), "refreshOn");
                assert_eq!(
                    plan.explicit_triggers[0].signal,
                    Some(item_id(lowered.module(), "usersChanged"))
                );
            }
            other => panic!("expected planned db.live lifecycle, found {other:?}"),
        }
    }

    #[test]
    fn custom_sources_keep_reactive_options_as_reconfiguration_inputs() {
        let lowered = lower_text(
            "custom_source_lifecycle.aivi",
            r#"
provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool

signal path = "/tmp/demo.txt"
signal enabled = True

@source custom.feed path with {
    activeWhen: enabled
}
signal updates : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "custom source lifecycle fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_lifecycles(lowered.module());
        let updates = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "updates")
            .expect("expected lifecycle plan for updates");

        match &updates.outcome {
            SourceLifecycleNodeOutcome::Planned(plan) => {
                assert_eq!(
                    plan.provider,
                    SourceProviderRef::Custom("custom.feed".into())
                );
                assert_eq!(
                    dependency_names(lowered.module(), &plan.reconfiguration_dependencies),
                    vec!["path".to_owned(), "enabled".to_owned()]
                );
                assert_eq!(plan.arguments.len(), 1);
                assert_eq!(plan.options.len(), 1);
                assert_eq!(plan.options[0].option_name.text(), "activeWhen");
                assert!(
                    plan.explicit_triggers.is_empty(),
                    "custom lifecycle elaboration must not invent built-in trigger bindings"
                );
                assert!(
                    plan.active_when.is_none(),
                    "custom lifecycle elaboration must not invent built-in activeWhen semantics"
                );
                assert_eq!(plan.cancellation, SourceCancellationPolicy::ProviderManaged);
                assert_eq!(
                    plan.stale_work,
                    SourceStaleWorkPolicy::DropStalePublications
                );
            }
            other => panic!("expected planned custom source lifecycle, found {other:?}"),
        }
    }

    #[test]
    fn blocks_missing_source_metadata() {
        let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
        assert!(
            !lowered.has_errors(),
            "source lifecycle blocking fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let mut module = lowered.module().clone();
        let signal_id = module
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(&module.items()[*item_id], Item::Signal(item) if item.name.text() == "users")
            })
            .expect("expected to find `users` signal item");
        let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
            panic!("expected `users` item to stay a signal");
        };
        signal.source_metadata = None;

        let report = elaborate_source_lifecycles(&module);
        let blocked = report
            .nodes()
            .iter()
            .find(|node| item_name(&module, node.owner) == "users")
            .expect("expected blocked lifecycle node");

        match &blocked.outcome {
            SourceLifecycleNodeOutcome::Blocked(node) => {
                assert!(
                    node.blockers
                        .contains(&SourceLifecycleElaborationBlocker::MissingSourceMetadata),
                    "expected missing source metadata blocker, found {:?}",
                    node.blockers
                );
            }
            other => panic!("expected blocked lifecycle node, found {other:?}"),
        }
    }
}
