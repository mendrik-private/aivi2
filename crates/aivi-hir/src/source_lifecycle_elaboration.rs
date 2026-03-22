use aivi_base::SourceSpan;
use aivi_typing::{SourceCancellationPolicy, SourceOptionWakeupCause};

use crate::{
    DecoratorId, DecoratorPayload, ExprId, ExprKind, Item, ItemId, Module, Name, SignalItem,
    SourceDecorator, SourceMetadata, SourceProviderRef,
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
    InvalidProviderShape { key: Box<str> },
}

pub fn elaborate_source_lifecycles(module: &Module) -> SourceLifecycleElaborationReport {
    let mut nodes = Vec::new();
    for (owner, item) in module.items().iter() {
        let Item::Signal(signal) = item else {
            continue;
        };
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
        })
        .collect()
}

fn active_when_binding(
    module: &Module,
    source: &SourceDecorator,
    provider: &SourceProviderRef,
) -> Option<SourceOptionSignalBinding> {
    let builtin_provider = provider.builtin()?;
    let Some(options) = source.options else {
        return None;
    };
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return None;
    };
    if builtin_provider.contract().option("activeWhen").is_none() {
        return None;
    }
    let field = record
        .fields
        .iter()
        .find(|field| field.label.text() == "activeWhen")?;
    Some(SourceOptionSignalBinding {
        option_span: field.span,
        option_name: field.label.clone(),
        expr: field.value,
    })
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
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{
        SourceCancellationPolicy, SourceLifecycleElaborationBlocker, SourceLifecycleNodeOutcome,
        SourceReplacementPolicy, SourceStaleWorkPolicy, SourceTeardownPolicy,
        elaborate_source_lifecycles,
    };
    use crate::{Item, SourceProviderRef, lower_module};

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
    }

    fn lower_text(path: &str, text: &str) -> crate::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn lower_fixture(path: &str) -> crate::LoweringResult {
        let text =
            fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
        lower_text(path, &text)
    }

    fn item_name(module: &crate::Module, item_id: crate::ItemId) -> &str {
        match &module.items()[item_id] {
            Item::Type(item) => item.name.text(),
            Item::Value(item) => item.name.text(),
            Item::Function(item) => item.name.text(),
            Item::Signal(item) => item.name.text(),
            Item::Class(item) => item.name.text(),
            Item::Domain(item) => item.name.text(),
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => "<anonymous>",
        }
    }

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

    #[test]
    fn elaborates_request_like_source_lifecycle_plans() {
        let lowered = lower_text(
            "request_source_lifecycle.aivi",
            r#"
domain Duration over Int
    literal s : Int -> Duration

sig apiHost = "https://api.example.com"
sig refresh = 0
sig enabled = True
sig pollInterval : Signal Duration = 5s

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
sig users : Signal Int
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
                assert_eq!(plan.explicit_triggers.len(), 1);
                assert_eq!(plan.explicit_triggers[0].option_name.text(), "refreshOn");
                let active_when = plan
                    .active_when
                    .as_ref()
                    .expect("http source should preserve activeWhen binding");
                assert_eq!(active_when.option_name.text(), "activeWhen");
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
    fn custom_sources_keep_reactive_options_as_reconfiguration_inputs() {
        let lowered = lower_text(
            "custom_source_lifecycle.aivi",
            r#"
provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool

sig path = "/tmp/demo.txt"
sig enabled = True

@source custom.feed path with {
    activeWhen: enabled
}
sig updates : Signal Int
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
