use std::{error::Error, fmt};

use aivi_base::SourceSpan;
use aivi_hir::{BindingId, ExprId, Name, NamePath, PatternId};
use aivi_runtime::{GraphBuildError, InputHandle, OwnerHandle, SignalGraph, SignalGraphBuilder};

use crate::plan::{
    AttributeSite, ChildOp, EventHookStrategy, EventHookTeardown, PlanNode, PlanNodeId,
    PlanNodeKind, PlanNodeTag, PropertyPlan, RepeatedChildPolicy, SetterSource, SetterTeardown,
    SetterUpdateStrategy, ShowMountPolicy, StableNodeId, StaticPropertyPlan, WidgetPlan,
};

/// Adapt one lowered widget plan into a runtime-facing assembly.
///
/// The current GTK/runtime seam chooses the narrowest coherent contract:
///
/// - every plan node becomes one runtime owner so stable identities survive intact,
/// - every dynamic property, event, or control expression site gets one owned input handle,
/// - child management stays localized through explicit owner references, and
/// - actual GTK object allocation remains out of scope for this slice.
pub fn assemble_widget_runtime(
    plan: &WidgetPlan,
) -> Result<WidgetRuntimeAssembly, WidgetRuntimeAdapterErrors> {
    WidgetRuntimeAssemblyBuilder::new(plan).build()
}

#[derive(Clone, Debug)]
pub struct WidgetRuntimeAssemblyBuilder<'a> {
    plan: &'a WidgetPlan,
}

impl<'a> WidgetRuntimeAssemblyBuilder<'a> {
    pub const fn new(plan: &'a WidgetPlan) -> Self {
        Self { plan }
    }

    pub fn build(self) -> Result<WidgetRuntimeAssembly, WidgetRuntimeAdapterErrors> {
        if let Err(error) = self.plan.validate() {
            return Err(WidgetRuntimeAdapterErrors::new(vec![
                WidgetRuntimeAdapterError::InvalidPlan(error),
            ]));
        }

        let (order, parent_plans) = self.validate_tree_shape()?;

        let mut graph_builder = SignalGraphBuilder::new();
        let mut plan_to_owner = vec![None; self.plan.len()];
        let mut owner_to_plan = Vec::with_capacity(self.plan.len());

        for &plan_id in &order {
            let node = self.plan.node(plan_id).ok_or_else(|| {
                WidgetRuntimeAdapterErrors::new(vec![WidgetRuntimeAdapterError::MissingPlanNode {
                    node: plan_id,
                }])
            })?;
            let parent = parent_plans[plan_id.index()]
                .map(|parent_plan| {
                    plan_to_owner[parent_plan.index()].ok_or(
                        WidgetRuntimeAdapterError::MissingOwnerForPlanNode { node: parent_plan },
                    )
                })
                .transpose()
                .map_err(|error| WidgetRuntimeAdapterErrors::new(vec![error]))?;
            let owner = graph_builder
                .add_owner(runtime_owner_name(node), parent)
                .map_err(|error| {
                    WidgetRuntimeAdapterErrors::new(vec![WidgetRuntimeAdapterError::GraphBuild(
                        error,
                    )])
                })?;
            plan_to_owner[plan_id.index()] = Some(owner);
            owner_to_plan.push(plan_id);
        }

        let plan_to_owner = plan_to_owner
            .into_iter()
            .enumerate()
            .map(|(index, owner)| {
                owner.ok_or(WidgetRuntimeAdapterError::MissingOwnerForPlanNode {
                    node: PlanNodeId::new(index as u32),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| WidgetRuntimeAdapterErrors::new(vec![error]))?;

        let mut errors = Vec::new();
        let mut nodes = vec![None; self.plan.len()];
        for &plan_id in &order {
            let node = self.plan.node(plan_id).ok_or_else(|| {
                WidgetRuntimeAdapterErrors::new(vec![WidgetRuntimeAdapterError::MissingPlanNode {
                    node: plan_id,
                }])
            })?;
            let owner = plan_to_owner[plan_id.index()];
            let parent = parent_plans[plan_id.index()].map(|parent_plan| RuntimeNodeRef {
                plan: parent_plan,
                owner: plan_to_owner[parent_plan.index()],
            });
            let kind = self.adapt_node_kind(
                plan_id,
                node,
                &plan_to_owner,
                &mut graph_builder,
                &mut errors,
            );
            nodes[plan_id.index()] = Some(RuntimePlanNode {
                plan: plan_id,
                stable_id: node.stable_id,
                span: node.span,
                owner,
                parent,
                kind,
            });
        }

        if !errors.is_empty() {
            return Err(WidgetRuntimeAdapterErrors::new(errors));
        }

        let graph = graph_builder.build().map_err(|error| {
            WidgetRuntimeAdapterErrors::new(vec![WidgetRuntimeAdapterError::GraphBuild(error)])
        })?;

        Ok(WidgetRuntimeAssembly {
            graph,
            root: self.plan.root(),
            plan_to_owner: plan_to_owner.into_boxed_slice(),
            owner_to_plan: owner_to_plan.into_boxed_slice(),
            nodes: nodes
                .into_iter()
                .enumerate()
                .map(|(index, node)| {
                    node.ok_or(WidgetRuntimeAdapterError::MissingPlanNode {
                        node: PlanNodeId::new(index as u32),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| WidgetRuntimeAdapterErrors::new(vec![error]))?
                .into_boxed_slice(),
        })
    }

    fn validate_tree_shape(
        &self,
    ) -> Result<(Vec<PlanNodeId>, Vec<Option<PlanNodeId>>), WidgetRuntimeAdapterErrors> {
        let mut marks = vec![VisitMark::NotSeen; self.plan.len()];
        let mut parents = vec![None; self.plan.len()];
        let mut order = Vec::with_capacity(self.plan.len());
        let mut errors = Vec::new();
        let mut stack = vec![TraversalEntry {
            node: self.plan.root(),
            parent: None,
            state: TraversalState::Enter,
        }];

        while let Some(entry) = stack.pop() {
            let Some(node) = self.plan.node(entry.node) else {
                errors.push(WidgetRuntimeAdapterError::MissingPlanNode { node: entry.node });
                continue;
            };
            let index = entry.node.index();
            match entry.state {
                TraversalState::Enter => match marks[index] {
                    VisitMark::NotSeen => {
                        marks[index] = VisitMark::Visiting;
                        parents[index] = entry.parent;
                        order.push(entry.node);
                        stack.push(TraversalEntry {
                            node: entry.node,
                            parent: entry.parent,
                            state: TraversalState::Exit,
                        });
                        push_children(node, entry.node, &mut stack);
                    }
                    VisitMark::Visiting => {
                        errors.push(WidgetRuntimeAdapterError::CyclicChildReference {
                            parent: entry.parent.unwrap_or(entry.node),
                            child: entry.node,
                        });
                    }
                    VisitMark::Visited => match (parents[index], entry.parent) {
                        (Some(first_parent), Some(second_parent))
                            if first_parent != second_parent =>
                        {
                            errors.push(WidgetRuntimeAdapterError::MultipleParents {
                                child: entry.node,
                                first_parent,
                                second_parent,
                            });
                        }
                        (Some(parent), Some(_)) => {
                            errors.push(WidgetRuntimeAdapterError::DuplicateChildReference {
                                parent,
                                child: entry.node,
                            });
                        }
                        (None, Some(parent)) => {
                            errors.push(WidgetRuntimeAdapterError::CyclicChildReference {
                                parent,
                                child: entry.node,
                            });
                        }
                        _ => {}
                    },
                },
                TraversalState::Exit => {
                    if matches!(marks[index], VisitMark::Visiting) {
                        marks[index] = VisitMark::Visited;
                    }
                }
            }
        }

        for (index, mark) in marks.into_iter().enumerate() {
            if matches!(mark, VisitMark::NotSeen) {
                errors.push(WidgetRuntimeAdapterError::UnreachableNode {
                    node: PlanNodeId::new(index as u32),
                });
            }
        }

        if errors.is_empty() {
            Ok((order, parents))
        } else {
            Err(WidgetRuntimeAdapterErrors::new(errors))
        }
    }

    fn adapt_node_kind(
        &self,
        plan_id: PlanNodeId,
        node: &PlanNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
        errors: &mut Vec<WidgetRuntimeAdapterError>,
    ) -> RuntimePlanNodeKind {
        match &node.kind {
            PlanNodeKind::Widget(widget) => RuntimePlanNodeKind::Widget(self.adapt_widget(
                plan_id,
                node.stable_id,
                widget,
                plan_to_owner,
                graph_builder,
                errors,
            )),
            PlanNodeKind::Group(group) => RuntimePlanNodeKind::Group(RuntimeGroupNode {
                widget: group.widget.clone(),
                group: group.group.clone(),
                children: adapt_child_ops(&group.children, plan_to_owner),
            }),
            PlanNodeKind::Show(show) => RuntimePlanNodeKind::Show(self.adapt_show(
                plan_id,
                node,
                show,
                plan_to_owner,
                graph_builder,
            )),
            PlanNodeKind::Each(each) => RuntimePlanNodeKind::Each(self.adapt_each(
                plan_id,
                node,
                each,
                plan_to_owner,
                graph_builder,
            )),
            PlanNodeKind::Empty(empty) => RuntimePlanNodeKind::Empty(RuntimeEmptyNode {
                children: adapt_child_ops(&empty.children, plan_to_owner),
            }),
            PlanNodeKind::Match(match_node) => RuntimePlanNodeKind::Match(self.adapt_match(
                plan_id,
                node,
                match_node,
                plan_to_owner,
                graph_builder,
                errors,
            )),
            PlanNodeKind::Case(case) => RuntimePlanNodeKind::Case(RuntimeCaseNode {
                pattern: case.pattern,
                children: adapt_child_ops(&case.children, plan_to_owner),
            }),
            PlanNodeKind::Fragment(fragment) => {
                RuntimePlanNodeKind::Fragment(RuntimeFragmentNode {
                    children: adapt_child_ops(&fragment.children, plan_to_owner),
                })
            }
            PlanNodeKind::With(with_node) => RuntimePlanNodeKind::With(self.adapt_with(
                plan_id,
                node,
                with_node,
                plan_to_owner,
                graph_builder,
            )),
        }
    }

    fn adapt_widget(
        &self,
        plan_id: PlanNodeId,
        stable_id: StableNodeId,
        widget: &crate::plan::WidgetNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
        errors: &mut Vec<WidgetRuntimeAdapterError>,
    ) -> RuntimeWidgetNode {
        let owner = plan_to_owner[plan_id.index()];
        let mut properties = Vec::with_capacity(widget.properties.len());
        for property in &widget.properties {
            match property {
                PropertyPlan::Static(static_property) => {
                    validate_attribute_site(plan_id, stable_id, &static_property.site, errors);
                    properties.push(RuntimePropertyBinding::Static(static_property.clone()));
                }
                PropertyPlan::Setter(setter) => {
                    validate_attribute_site(plan_id, stable_id, &setter.site, errors);
                    let input = graph_builder
                        .add_input(runtime_setter_name(stable_id, setter), Some(owner))
                        .expect("runtime owner handles were validated before setter allocation");
                    properties.push(RuntimePropertyBinding::Setter(RuntimeSetterBinding {
                        site: setter.site.clone(),
                        name: setter.name.clone(),
                        source: setter.source.clone(),
                        owner,
                        input,
                        update: setter.update,
                        teardown: setter.teardown,
                    }));
                }
            }
        }

        let mut event_hooks = Vec::with_capacity(widget.event_hooks.len());
        for event in &widget.event_hooks {
            validate_attribute_site(plan_id, stable_id, &event.site, errors);
            let input = graph_builder
                .add_input(runtime_event_name(stable_id, event), Some(owner))
                .expect("runtime owner handles were validated before event allocation");
            event_hooks.push(RuntimeEventBinding {
                site: event.site.clone(),
                name: event.name.clone(),
                handler: event.handler,
                owner,
                input,
                hookup: event.hookup,
                teardown: event.teardown,
            });
        }

        RuntimeWidgetNode {
            widget: widget.widget.clone(),
            properties: properties.into_boxed_slice(),
            event_hooks: event_hooks.into_boxed_slice(),
            children: adapt_child_ops(&widget.children, plan_to_owner),
        }
    }

    fn adapt_show(
        &self,
        plan_id: PlanNodeId,
        node: &PlanNode,
        show: &crate::plan::ShowNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
    ) -> RuntimeShowNode {
        let owner = plan_to_owner[plan_id.index()];
        let when = RuntimeExprInput {
            owner,
            expr: show.when,
            input: graph_builder
                .add_input(runtime_control_name(node.stable_id, "when"), Some(owner))
                .expect("runtime owner handles were validated before control input allocation"),
        };
        let mount = match show.mount {
            ShowMountPolicy::UnmountWhenHidden => RuntimeShowMountPolicy::UnmountWhenHidden,
            ShowMountPolicy::KeepMounted { decision } => RuntimeShowMountPolicy::KeepMounted {
                decision: RuntimeExprInput {
                    owner,
                    expr: decision,
                    input: graph_builder
                        .add_input(
                            runtime_control_name(node.stable_id, "keep-mounted"),
                            Some(owner),
                        )
                        .expect(
                            "runtime owner handles were validated before keep-mounted allocation",
                        ),
                },
            },
        };
        RuntimeShowNode {
            when,
            mount,
            children: adapt_child_ops(&show.children, plan_to_owner),
        }
    }

    fn adapt_each(
        &self,
        plan_id: PlanNodeId,
        node: &PlanNode,
        each: &crate::plan::EachNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
    ) -> RuntimeEachNode {
        let owner = plan_to_owner[plan_id.index()];
        let collection = RuntimeExprInput {
            owner,
            expr: each.collection,
            input: graph_builder
                .add_input(
                    runtime_control_name(node.stable_id, "collection"),
                    Some(owner),
                )
                .expect("runtime owner handles were validated before collection allocation"),
        };
        let key_input = match &each.child_policy {
            RepeatedChildPolicy::Positional { .. } => None,
            RepeatedChildPolicy::Keyed { key, .. } => Some(RuntimeExprInput {
                owner,
                expr: *key,
                input: graph_builder
                    .add_input(runtime_control_name(node.stable_id, "key"), Some(owner))
                    .expect("runtime owner handles were validated before key allocation"),
            }),
        };
        RuntimeEachNode {
            collection,
            key_input,
            binding: each.binding,
            child_policy: each.child_policy.clone(),
            item_children: adapt_child_ops(&each.item_children, plan_to_owner),
            empty_branch: each
                .empty_branch
                .map(|branch| node_ref(branch, plan_to_owner)),
        }
    }

    fn adapt_match(
        &self,
        plan_id: PlanNodeId,
        node: &PlanNode,
        match_node: &crate::plan::MatchNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
        errors: &mut Vec<WidgetRuntimeAdapterError>,
    ) -> RuntimeMatchNode {
        let owner = plan_to_owner[plan_id.index()];
        let cases = match_node
            .cases
            .iter()
            .copied()
            .filter_map(|case_plan| match self.plan.node(case_plan) {
                Some(PlanNode {
                    kind: PlanNodeKind::Case(case),
                    ..
                }) => Some(RuntimeCaseBranch {
                    case: node_ref(case_plan, plan_to_owner),
                    pattern: case.pattern,
                }),
                Some(case_node) => {
                    errors.push(WidgetRuntimeAdapterError::UnexpectedMatchCaseBranch {
                        match_node: plan_id,
                        case: case_plan,
                        found: case_node.kind.tag(),
                    });
                    None
                }
                None => {
                    errors.push(WidgetRuntimeAdapterError::MissingPlanNode { node: case_plan });
                    None
                }
            })
            .collect::<Vec<_>>();
        RuntimeMatchNode {
            scrutinee: RuntimeExprInput {
                owner,
                expr: match_node.scrutinee,
                input: graph_builder
                    .add_input(
                        runtime_control_name(node.stable_id, "scrutinee"),
                        Some(owner),
                    )
                    .expect("runtime owner handles were validated before scrutinee allocation"),
            },
            cases: cases.into_boxed_slice(),
        }
    }

    fn adapt_with(
        &self,
        plan_id: PlanNodeId,
        node: &PlanNode,
        with_node: &crate::plan::WithNode,
        plan_to_owner: &[OwnerHandle],
        graph_builder: &mut SignalGraphBuilder,
    ) -> RuntimeWithNode {
        let owner = plan_to_owner[plan_id.index()];
        RuntimeWithNode {
            value: RuntimeExprInput {
                owner,
                expr: with_node.value,
                input: graph_builder
                    .add_input(runtime_control_name(node.stable_id, "value"), Some(owner))
                    .expect("runtime owner handles were validated before with-value allocation"),
            },
            binding: with_node.binding,
            children: adapt_child_ops(&with_node.children, plan_to_owner),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetRuntimeAssembly {
    graph: SignalGraph,
    root: PlanNodeId,
    plan_to_owner: Box<[OwnerHandle]>,
    owner_to_plan: Box<[PlanNodeId]>,
    nodes: Box<[RuntimePlanNode]>,
}

impl WidgetRuntimeAssembly {
    pub fn graph(&self) -> &SignalGraph {
        &self.graph
    }

    pub const fn root(&self) -> PlanNodeId {
        self.root
    }

    pub fn nodes(&self) -> &[RuntimePlanNode] {
        &self.nodes
    }

    pub fn node(&self, plan: PlanNodeId) -> Option<&RuntimePlanNode> {
        self.nodes.get(plan.index())
    }

    pub fn owner(&self, plan: PlanNodeId) -> Option<OwnerHandle> {
        self.plan_to_owner.get(plan.index()).copied()
    }

    pub fn node_ref(&self, plan: PlanNodeId) -> Option<RuntimeNodeRef> {
        Some(RuntimeNodeRef {
            plan,
            owner: self.owner(plan)?,
        })
    }

    pub fn plan_for_owner(&self, owner: OwnerHandle) -> Option<PlanNodeId> {
        self.owner_to_plan.get(owner.as_raw() as usize).copied()
    }

    pub fn node_for_owner(&self, owner: OwnerHandle) -> Option<&RuntimePlanNode> {
        self.plan_for_owner(owner).and_then(|plan| self.node(plan))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeNodeRef {
    pub plan: PlanNodeId,
    pub owner: OwnerHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePlanNode {
    pub plan: PlanNodeId,
    pub stable_id: StableNodeId,
    pub span: SourceSpan,
    pub owner: OwnerHandle,
    pub parent: Option<RuntimeNodeRef>,
    pub kind: RuntimePlanNodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimePlanNodeKind {
    Widget(RuntimeWidgetNode),
    Group(RuntimeGroupNode),
    Show(RuntimeShowNode),
    Each(RuntimeEachNode),
    Empty(RuntimeEmptyNode),
    Match(RuntimeMatchNode),
    Case(RuntimeCaseNode),
    Fragment(RuntimeFragmentNode),
    With(RuntimeWithNode),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWidgetNode {
    pub widget: NamePath,
    pub properties: Box<[RuntimePropertyBinding]>,
    pub event_hooks: Box<[RuntimeEventBinding]>,
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeGroupNode {
    pub widget: NamePath,
    pub group: Name,
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimePropertyBinding {
    Static(StaticPropertyPlan),
    Setter(RuntimeSetterBinding),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSetterBinding {
    pub site: AttributeSite,
    pub name: Name,
    pub source: SetterSource,
    pub owner: OwnerHandle,
    pub input: InputHandle,
    pub update: SetterUpdateStrategy,
    pub teardown: SetterTeardown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEventBinding {
    pub site: AttributeSite,
    pub name: Name,
    pub handler: ExprId,
    pub owner: OwnerHandle,
    pub input: InputHandle,
    pub hookup: EventHookStrategy,
    pub teardown: EventHookTeardown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeShowNode {
    pub when: RuntimeExprInput,
    pub mount: RuntimeShowMountPolicy,
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeShowMountPolicy {
    UnmountWhenHidden,
    KeepMounted { decision: RuntimeExprInput },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEachNode {
    pub collection: RuntimeExprInput,
    pub key_input: Option<RuntimeExprInput>,
    pub binding: BindingId,
    pub child_policy: RepeatedChildPolicy,
    pub item_children: Box<[RuntimeChildOp]>,
    pub empty_branch: Option<RuntimeNodeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEmptyNode {
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeMatchNode {
    pub scrutinee: RuntimeExprInput,
    pub cases: Box<[RuntimeCaseBranch]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCaseBranch {
    pub case: RuntimeNodeRef,
    pub pattern: PatternId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCaseNode {
    pub pattern: PatternId,
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFragmentNode {
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWithNode {
    pub value: RuntimeExprInput,
    pub binding: BindingId,
    pub children: Box<[RuntimeChildOp]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeExprInput {
    pub owner: OwnerHandle,
    pub expr: ExprId,
    pub input: InputHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeChildOp {
    Append(RuntimeNodeRef),
}

impl RuntimeChildOp {
    pub const fn child(self) -> RuntimeNodeRef {
        match self {
            Self::Append(child) => child,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetRuntimeAdapterErrors {
    errors: Box<[WidgetRuntimeAdapterError]>,
}

impl WidgetRuntimeAdapterErrors {
    pub fn new(errors: Vec<WidgetRuntimeAdapterError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[WidgetRuntimeAdapterError] {
        &self.errors
    }
}

impl fmt::Display for WidgetRuntimeAdapterErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "failed to adapt widget plan into a runtime assembly:")?;
        for error in &self.errors {
            writeln!(f, "- {error}")?;
        }
        Ok(())
    }
}

impl Error for WidgetRuntimeAdapterErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetRuntimeAdapterError {
    InvalidPlan(crate::plan::PlanValidationError),
    MissingPlanNode {
        node: PlanNodeId,
    },
    DuplicateChildReference {
        parent: PlanNodeId,
        child: PlanNodeId,
    },
    MultipleParents {
        child: PlanNodeId,
        first_parent: PlanNodeId,
        second_parent: PlanNodeId,
    },
    CyclicChildReference {
        parent: PlanNodeId,
        child: PlanNodeId,
    },
    UnreachableNode {
        node: PlanNodeId,
    },
    MissingOwnerForPlanNode {
        node: PlanNodeId,
    },
    AttributeSiteOwnerMismatch {
        node: PlanNodeId,
        expected: StableNodeId,
        found: StableNodeId,
    },
    UnexpectedMatchCaseBranch {
        match_node: PlanNodeId,
        case: PlanNodeId,
        found: PlanNodeTag,
    },
    GraphBuild(GraphBuildError),
}

impl fmt::Display for WidgetRuntimeAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(error) => write!(f, "widget plan is invalid: {error}"),
            Self::MissingPlanNode { node } => write!(f, "widget plan node {node} is missing"),
            Self::DuplicateChildReference { parent, child } => write!(
                f,
                "widget plan parent {parent} references child {child} more than once"
            ),
            Self::MultipleParents {
                child,
                first_parent,
                second_parent,
            } => write!(
                f,
                "widget plan child {child} is referenced by both {first_parent} and {second_parent}"
            ),
            Self::CyclicChildReference { parent, child } => write!(
                f,
                "widget plan parent {parent} introduces a cycle through child {child}"
            ),
            Self::UnreachableNode { node } => {
                write!(
                    f,
                    "widget plan node {node} is unreachable from the plan root"
                )
            }
            Self::MissingOwnerForPlanNode { node } => {
                write!(
                    f,
                    "widget plan node {node} does not have a runtime owner binding"
                )
            }
            Self::AttributeSiteOwnerMismatch {
                node,
                expected,
                found,
            } => write!(
                f,
                "widget plan node {node} expected attribute site owner {expected:?}, found {found:?}"
            ),
            Self::UnexpectedMatchCaseBranch {
                match_node,
                case,
                found,
            } => write!(
                f,
                "widget plan match node {match_node} expected case branch {case} to be Case, found {found:?}"
            ),
            Self::GraphBuild(error) => write!(f, "signal graph build failed: {error:?}"),
        }
    }
}

impl Error for WidgetRuntimeAdapterError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraversalState {
    Enter,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TraversalEntry {
    node: PlanNodeId,
    parent: Option<PlanNodeId>,
    state: TraversalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisitMark {
    NotSeen,
    Visiting,
    Visited,
}

fn push_children(node: &PlanNode, parent: PlanNodeId, stack: &mut Vec<TraversalEntry>) {
    let mut push_child = |child: PlanNodeId| {
        stack.push(TraversalEntry {
            node: child,
            parent: Some(parent),
            state: TraversalState::Enter,
        });
    };

    match &node.kind {
        PlanNodeKind::Widget(widget) => {
            for child in widget.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Group(group) => {
            for child in group.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Show(show) => {
            for child in show.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Each(each) => {
            if let Some(empty) = each.empty_branch {
                push_child(empty);
            }
            for child in each.item_children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Empty(empty) => {
            for child in empty.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Match(match_node) => {
            for case in match_node.cases.iter().rev().copied() {
                push_child(case);
            }
        }
        PlanNodeKind::Case(case) => {
            for child in case.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::Fragment(fragment) => {
            for child in fragment.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
        PlanNodeKind::With(with_node) => {
            for child in with_node.children.iter().rev().copied() {
                push_child(child.child());
            }
        }
    }
}

fn validate_attribute_site(
    plan: PlanNodeId,
    expected: StableNodeId,
    site: &AttributeSite,
    errors: &mut Vec<WidgetRuntimeAdapterError>,
) {
    if site.owner != expected {
        errors.push(WidgetRuntimeAdapterError::AttributeSiteOwnerMismatch {
            node: plan,
            expected,
            found: site.owner,
        });
    }
}

fn adapt_child_ops(children: &[ChildOp], plan_to_owner: &[OwnerHandle]) -> Box<[RuntimeChildOp]> {
    children
        .iter()
        .copied()
        .map(|child| RuntimeChildOp::Append(node_ref(child.child(), plan_to_owner)))
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn node_ref(plan: PlanNodeId, plan_to_owner: &[OwnerHandle]) -> RuntimeNodeRef {
    RuntimeNodeRef {
        plan,
        owner: plan_to_owner[plan.index()],
    }
}

fn runtime_owner_name(node: &PlanNode) -> Box<str> {
    format!(
        "{}:{}",
        runtime_tag_label(node.kind.tag()),
        runtime_stable_name(node.stable_id)
    )
    .into_boxed_str()
}

fn runtime_setter_name(
    stable_id: StableNodeId,
    setter: &crate::plan::SetterBindingPlan,
) -> Box<str> {
    format!(
        "{}#setter:{}@{}",
        runtime_stable_name(stable_id),
        setter.name.text(),
        setter.site.index
    )
    .into_boxed_str()
}

fn runtime_event_name(stable_id: StableNodeId, event: &crate::plan::EventHookPlan) -> Box<str> {
    format!(
        "{}#event:{}@{}",
        runtime_stable_name(stable_id),
        event.name.text(),
        event.site.index
    )
    .into_boxed_str()
}

fn runtime_control_name(stable_id: StableNodeId, slot: &str) -> Box<str> {
    format!("{}#control:{slot}", runtime_stable_name(stable_id)).into_boxed_str()
}

fn runtime_stable_name(stable_id: StableNodeId) -> String {
    match stable_id {
        StableNodeId::Markup(id) => format!("markup:{}", id.as_raw()),
        StableNodeId::Control(id) => format!("control:{}", id.as_raw()),
    }
}

fn runtime_tag_label(tag: PlanNodeTag) -> &'static str {
    match tag {
        PlanNodeTag::Widget => "widget",
        PlanNodeTag::Group => "group",
        PlanNodeTag::Show => "show",
        PlanNodeTag::Each => "each",
        PlanNodeTag::Empty => "empty",
        PlanNodeTag::Match => "match",
        PlanNodeTag::Case => "case",
        PlanNodeTag::Fragment => "fragment",
        PlanNodeTag::With => "with",
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::{FileId, SourceDatabase, SourceSpan, Span};
    use aivi_hir::{
        ControlNodeId, ExprKind, Item, MarkupNodeId, Name, NamePath, TextLiteral, lower_module,
    };
    use aivi_syntax::parse_module;

    use super::*;
    use crate::ChildUpdateMode;
    use crate::{ChildOp, PlanNode, StableNodeId, StaticPropertyValue, lower_markup_expr};

    fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
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

    fn find_value_item<'a>(module: &'a aivi_hir::Module, name: &str) -> &'a aivi_hir::ValueItem {
        module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(value) if value.name.text() == name => Some(value),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected to find value item `{name}`"))
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
    }

    fn span() -> SourceSpan {
        SourceSpan::new(FileId::new(0), Span::from(0..1))
    }

    fn widget_name(text: &str) -> NamePath {
        NamePath::from_vec(vec![Name::new(text, span()).expect("name should be valid")])
            .expect("path should be valid")
    }

    fn empty_text() -> TextLiteral {
        TextLiteral {
            segments: Vec::new(),
        }
    }

    #[test]
    fn assembles_property_and_event_input_handles_for_widget_sites() {
        let hir = lower_text(
            "runtime-event-attrs.aivi",
            r#"
value isVisible = True
value clickHandler = True
value view =
    <Button label="Save" visible={isVisible} onClick={clickHandler} />
"#,
        );
        assert!(
            !hir.has_errors(),
            "HIR lowering should succeed before GTK lowering: {:?}",
            hir.diagnostics()
        );

        let module = hir.module();
        let value = find_value_item(module, "view");
        let ExprKind::Markup(_) = module.exprs()[value.body].kind else {
            panic!("view should lower from markup");
        };

        let plan = lower_markup_expr(module, value.body).expect("markup should lower");
        let assembly =
            assemble_widget_runtime(&plan).expect("lowered widget plan should adapt cleanly");

        assert_eq!(assembly.graph().owner_count(), 1);
        assert_eq!(assembly.graph().signal_count(), 2);

        let root = assembly
            .node(plan.root())
            .expect("root runtime node should exist");
        assert_eq!(assembly.plan_for_owner(root.owner), Some(plan.root()));
        let RuntimePlanNodeKind::Widget(widget) = &root.kind else {
            panic!("expected runtime widget node, found {:?}", root.kind);
        };
        assert_eq!(widget.widget.to_string(), "Button");
        assert_eq!(widget.properties.len(), 2);
        assert_eq!(widget.event_hooks.len(), 1);
        assert!(matches!(
            &widget.properties[0],
            RuntimePropertyBinding::Static(static_prop)
                if static_prop.name.text() == "label"
        ));
        assert!(matches!(
            &widget.properties[1],
            RuntimePropertyBinding::Setter(setter)
                if setter.name.text() == "visible"
                    && setter.input.as_raw() == 0
        ));
        assert_eq!(widget.event_hooks[0].name.text(), "onClick");
        assert_eq!(widget.event_hooks[0].input.as_raw(), 1);
    }

    #[test]
    fn assembles_owner_hierarchy_and_child_management_for_control_fixture() {
        let fixture = fixture_root()
            .join("milestone-2")
            .join("valid")
            .join("markup-control-nodes")
            .join("main.aivi");
        let hir = lower_text(
            fixture.to_string_lossy().as_ref(),
            &fs::read_to_string(&fixture).expect("fixture should be readable"),
        );
        assert!(
            !hir.has_errors(),
            "fixture should lower into HIR cleanly: {:?}",
            hir.diagnostics()
        );

        let module = hir.module();
        let value = find_value_item(module, "screenView");
        let plan = lower_markup_expr(module, value.body).expect("fixture markup should lower");
        let assembly =
            assemble_widget_runtime(&plan).expect("control fixture should adapt cleanly");

        assert_eq!(assembly.graph().owner_count(), plan.len());
        assert_eq!(assembly.graph().signal_count(), 9);

        let root = assembly.node(plan.root()).expect("root node should exist");
        let RuntimePlanNodeKind::Fragment(fragment) = &root.kind else {
            panic!("expected fragment runtime root, found {:?}", root.kind);
        };
        assert_eq!(fragment.children.len(), 2);
        assert_eq!(
            assembly
                .graph()
                .owner(root.owner)
                .expect("root owner should exist")
                .children()
                .len(),
            2
        );

        let show_ref = fragment.children[1].child();
        let show = assembly
            .node(show_ref.plan)
            .expect("show runtime node should exist");
        let RuntimePlanNodeKind::Show(show_node) = &show.kind else {
            panic!("expected show runtime node, found {:?}", show.kind);
        };
        assert_eq!(show_node.when.input.as_raw(), 1);
        assert!(matches!(
            show_node.mount,
            RuntimeShowMountPolicy::KeepMounted { .. }
        ));
        let RuntimeShowMountPolicy::KeepMounted { decision } = &show_node.mount else {
            unreachable!();
        };
        assert_eq!(decision.input.as_raw(), 2);

        let with_ref = show_node.children[0].child();
        let with = assembly
            .node(with_ref.plan)
            .expect("with runtime node should exist");
        let RuntimePlanNodeKind::With(with_node) = &with.kind else {
            panic!("expected with runtime node, found {:?}", with.kind);
        };
        assert_eq!(with_node.value.input.as_raw(), 3);

        let match_ref = with_node.children[0].child();
        let match_node = assembly
            .node(match_ref.plan)
            .expect("match runtime node should exist");
        let RuntimePlanNodeKind::Match(match_node) = &match_node.kind else {
            panic!("expected match runtime node, found {:?}", match_node.kind);
        };
        assert_eq!(match_node.scrutinee.input.as_raw(), 4);
        assert_eq!(match_node.cases.len(), 3);

        let ready_case = &match_node.cases[1];
        let ready = assembly
            .node(ready_case.case.plan)
            .expect("ready case node should exist");
        let RuntimePlanNodeKind::Case(ready_case_node) = &ready.kind else {
            panic!("expected case runtime node, found {:?}", ready.kind);
        };
        let each_ref = ready_case_node.children[0].child();
        let each = assembly
            .node(each_ref.plan)
            .expect("each runtime node should exist");
        let RuntimePlanNodeKind::Each(each_node) = &each.kind else {
            panic!("expected each runtime node, found {:?}", each.kind);
        };
        assert_eq!(each_node.collection.input.as_raw(), 5);
        let key_input = each_node
            .key_input
            .as_ref()
            .expect("keyed each nodes should allocate a runtime key input");
        assert_eq!(key_input.input.as_raw(), 6);
        assert!(matches!(
            each_node.child_policy,
            RepeatedChildPolicy::Keyed {
                updates: ChildUpdateMode::Localized,
                ..
            }
        ));
        assert!(each_node.empty_branch.is_some());

        let row_ref = each_node.item_children[0].child();
        let row = assembly.node(row_ref.plan).expect("row node should exist");
        let RuntimePlanNodeKind::Widget(row_widget) = &row.kind else {
            panic!("expected row widget runtime node, found {:?}", row.kind);
        };
        let RuntimePropertyBinding::Setter(row_title) = &row_widget.properties[0] else {
            panic!("expected runtime setter for row title");
        };
        assert_eq!(row_title.input.as_raw(), 7);

        let failed_case = &match_node.cases[2];
        let failed = assembly
            .node(failed_case.case.plan)
            .expect("failed case node should exist");
        let RuntimePlanNodeKind::Case(failed_case_node) = &failed.kind else {
            panic!("expected case runtime node, found {:?}", failed.kind);
        };
        let failed_label_ref = failed_case_node.children[0].child();
        let failed_label = assembly
            .node(failed_label_ref.plan)
            .expect("failed label node should exist");
        let RuntimePlanNodeKind::Widget(failed_label_widget) = &failed_label.kind else {
            panic!(
                "expected failed label widget runtime node, found {:?}",
                failed_label.kind
            );
        };
        let RuntimePropertyBinding::Setter(failed_text) = &failed_label_widget.properties[0] else {
            panic!("expected runtime setter for failed label");
        };
        assert_eq!(failed_text.input.as_raw(), 8);
        assert!(matches!(
            failed_text.source,
            SetterSource::InterpolatedText(_)
        ));
    }

    #[test]
    fn rejects_nodes_with_multiple_parents() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![
                PlanNode {
                    stable_id: StableNodeId::Markup(MarkupNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Fragment(crate::plan::FragmentNode {
                        children: vec![
                            ChildOp::Append(PlanNodeId::new(1)),
                            ChildOp::Append(PlanNodeId::new(2)),
                        ],
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Markup(MarkupNodeId::from_raw(1)),
                    span: span(),
                    kind: PlanNodeKind::Widget(crate::plan::WidgetNode {
                        widget: widget_name("Label"),
                        properties: vec![PropertyPlan::Static(StaticPropertyPlan {
                            site: AttributeSite {
                                owner: StableNodeId::Markup(MarkupNodeId::from_raw(1)),
                                index: 0,
                                span: span(),
                            },
                            name: Name::new("text", span()).expect("name should be valid"),
                            value: StaticPropertyValue::Text(empty_text()),
                        })],
                        event_hooks: Vec::new(),
                        children: Vec::new(),
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Control(ControlNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Fragment(crate::plan::FragmentNode {
                        children: vec![ChildOp::Append(PlanNodeId::new(1))],
                    }),
                },
            ],
        );

        let errors = assemble_widget_runtime(&plan).expect_err("shared child should be rejected");
        assert!(errors.errors().iter().any(|error| {
            matches!(
                error,
                WidgetRuntimeAdapterError::MultipleParents {
                    child,
                    first_parent,
                    second_parent,
                } if *child == PlanNodeId::new(1)
                    && *first_parent == PlanNodeId::new(0)
                    && *second_parent == PlanNodeId::new(2)
            )
        }));
    }

    #[test]
    fn rejects_unreachable_plan_nodes() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![
                PlanNode {
                    stable_id: StableNodeId::Markup(MarkupNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Widget(crate::plan::WidgetNode {
                        widget: widget_name("Label"),
                        properties: vec![PropertyPlan::Static(StaticPropertyPlan {
                            site: AttributeSite {
                                owner: StableNodeId::Markup(MarkupNodeId::from_raw(0)),
                                index: 0,
                                span: span(),
                            },
                            name: Name::new("text", span()).expect("name should be valid"),
                            value: StaticPropertyValue::Text(empty_text()),
                        })],
                        event_hooks: Vec::new(),
                        children: Vec::new(),
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Markup(MarkupNodeId::from_raw(1)),
                    span: span(),
                    kind: PlanNodeKind::Widget(crate::plan::WidgetNode {
                        widget: widget_name("Label"),
                        properties: Vec::new(),
                        event_hooks: Vec::new(),
                        children: Vec::new(),
                    }),
                },
            ],
        );

        let errors =
            assemble_widget_runtime(&plan).expect_err("unreachable widget plan nodes must fail");
        assert!(errors.errors().iter().any(|error| {
            matches!(
                error,
                WidgetRuntimeAdapterError::UnreachableNode { node }
                    if *node == PlanNodeId::new(1)
            )
        }));
    }
}
