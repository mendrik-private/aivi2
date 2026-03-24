use std::{collections::BTreeSet, error::Error, fmt};

use aivi_base::SourceSpan;
use aivi_hir::{BindingId, NamePath, PatternId};
use aivi_runtime::{OwnerHandle, SignalGraph};

use crate::{
    plan::{PlanNodeId, PlanNodeTag, RepeatedChildPolicy, StableNodeId, WidgetPlan},
    runtime_adapter::{
        RuntimeCaseBranch, RuntimeChildOp, RuntimeEventBinding, RuntimeExprInput, RuntimeNodeRef,
        RuntimePlanNode, RuntimePlanNodeKind, RuntimePropertyBinding, RuntimeShowMountPolicy,
        WidgetRuntimeAdapterError, WidgetRuntimeAdapterErrors, WidgetRuntimeAssembly,
        assemble_widget_runtime,
    },
};

/// Lower one widget plan through runtime assembly into a GTK-oriented executable bridge graph.
///
/// This layer keeps the contract intentionally narrow and explicit:
///
/// - widget nodes expose their schema-declared default child group; widgets with richer multi-slot
///   child schemas still need future markup syntax before they can be addressed,
/// - control nodes lower to concrete body/case/empty groups instead of opaque child metadata, and
/// - localized child edits stay explicit so keyed collection updates cannot regress into VDOM-style
///   diffing.
pub fn lower_widget_bridge(plan: &WidgetPlan) -> Result<GtkBridgeGraph, GtkBridgeLoweringErrors> {
    let assembly = assemble_widget_runtime(plan).map_err(GtkBridgeLoweringErrors::from)?;
    lower_widget_bridge_from_assembly(assembly)
}

/// Lower one runtime assembly into the GTK-oriented bridge graph.
pub fn lower_widget_bridge_from_assembly(
    assembly: WidgetRuntimeAssembly,
) -> Result<GtkBridgeGraph, GtkBridgeLoweringErrors> {
    GtkBridgeGraphBuilder::new(assembly).build()
}

#[derive(Clone, Debug)]
pub struct GtkBridgeGraphBuilder {
    assembly: WidgetRuntimeAssembly,
}

impl GtkBridgeGraphBuilder {
    pub const fn new(assembly: WidgetRuntimeAssembly) -> Self {
        Self { assembly }
    }

    pub fn build(self) -> Result<GtkBridgeGraph, GtkBridgeLoweringErrors> {
        let root_plan = self.assembly.root();
        let root = self
            .assembly
            .node_ref(root_plan)
            .map(GtkBridgeNodeRef::from)
            .ok_or_else(|| {
                GtkBridgeLoweringErrors::new(vec![GtkBridgeLoweringError::MissingRuntimeNode {
                    node: root_plan,
                }])
            })?;

        let mut errors = Vec::new();
        match self.assembly.node(root_plan) {
            Some(root_node) => {
                if let Some(parent) = root_node.parent {
                    errors.push(GtkBridgeLoweringError::RootHasParent {
                        root: root_plan,
                        parent: parent.plan,
                    });
                }
            }
            None => errors.push(GtkBridgeLoweringError::MissingRuntimeNode { node: root_plan }),
        }

        let mut nodes = Vec::with_capacity(self.assembly.nodes().len());
        for node in self.assembly.nodes() {
            if node.plan != root_plan && node.parent.is_none() {
                errors.push(GtkBridgeLoweringError::AdditionalRoot {
                    expected: root_plan,
                    additional: node.plan,
                });
            }
            nodes.push(self.build_node(node, &mut errors));
        }

        if errors.is_empty() {
            Ok(GtkBridgeGraph {
                assembly: self.assembly,
                root,
                nodes: nodes.into_boxed_slice(),
            })
        } else {
            Err(GtkBridgeLoweringErrors::new(errors))
        }
    }

    fn build_node(
        &self,
        node: &RuntimePlanNode,
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) -> GtkBridgeNode {
        let node_ref = GtkBridgeNodeRef::from(RuntimeNodeRef {
            plan: node.plan,
            owner: node.owner,
        });
        GtkBridgeNode {
            plan: node.plan,
            stable_id: node.stable_id,
            span: node.span,
            owner: node.owner,
            parent: node.parent.map(GtkBridgeNodeRef::from),
            kind: self.build_node_kind(node_ref, &node.kind, errors),
        }
    }

    fn build_node_kind(
        &self,
        node_ref: GtkBridgeNodeRef,
        kind: &RuntimePlanNodeKind,
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) -> GtkBridgeNodeKind {
        match kind {
            RuntimePlanNodeKind::Widget(widget) => GtkBridgeNodeKind::Widget(GtkWidgetNode {
                widget: widget.widget.clone(),
                properties: widget.properties.clone(),
                event_hooks: widget.event_hooks.clone(),
                default_children: self.child_group(
                    node_ref,
                    GtkChildGroupKind::WidgetDefault,
                    &widget.children,
                    errors,
                ),
            }),
            RuntimePlanNodeKind::Show(show) => GtkBridgeNodeKind::Show(GtkShowNode {
                when: show.when.clone(),
                mount: show.mount.clone(),
                body: self.child_group(
                    node_ref,
                    GtkChildGroupKind::ShowBody,
                    &show.children,
                    errors,
                ),
            }),
            RuntimePlanNodeKind::Each(each) => GtkBridgeNodeKind::Each(GtkEachNode {
                collection: each.collection.clone(),
                key_input: each.key_input.clone(),
                binding: each.binding,
                child_policy: each.child_policy.clone(),
                item_template: self.child_group(
                    node_ref,
                    GtkChildGroupKind::EachItemTemplate,
                    &each.item_children,
                    errors,
                ),
                empty_branch: each
                    .empty_branch
                    .and_then(|empty| self.build_empty_branch(node_ref, empty, errors)),
            }),
            RuntimePlanNodeKind::Empty(empty) => GtkBridgeNodeKind::Empty(GtkEmptyNode {
                body: self.child_group(
                    node_ref,
                    GtkChildGroupKind::EmptyBody,
                    &empty.children,
                    errors,
                ),
            }),
            RuntimePlanNodeKind::Match(match_node) => GtkBridgeNodeKind::Match(GtkMatchNode {
                scrutinee: match_node.scrutinee.clone(),
                cases: match_node
                    .cases
                    .iter()
                    .filter_map(|branch| self.build_case_branch(node_ref, branch, errors))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            }),
            RuntimePlanNodeKind::Case(case) => GtkBridgeNodeKind::Case(GtkCaseNode {
                pattern: case.pattern,
                body: self.child_group(
                    node_ref,
                    GtkChildGroupKind::CaseBody,
                    &case.children,
                    errors,
                ),
            }),
            RuntimePlanNodeKind::Fragment(fragment) => {
                GtkBridgeNodeKind::Fragment(GtkFragmentNode {
                    body: self.child_group(
                        node_ref,
                        GtkChildGroupKind::FragmentBody,
                        &fragment.children,
                        errors,
                    ),
                })
            }
            RuntimePlanNodeKind::With(with_node) => GtkBridgeNodeKind::With(GtkWithNode {
                value: with_node.value.clone(),
                binding: with_node.binding,
                body: self.child_group(
                    node_ref,
                    GtkChildGroupKind::WithBody,
                    &with_node.children,
                    errors,
                ),
            }),
        }
    }

    fn build_case_branch(
        &self,
        match_ref: GtkBridgeNodeRef,
        branch: &RuntimeCaseBranch,
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) -> Option<GtkCaseBranch> {
        self.validate_direct_child(match_ref, branch.case.into(), errors);
        let case_ref = GtkBridgeNodeRef::from(branch.case);
        let case_node = self.assembly.node(branch.case.plan);
        match case_node {
            Some(RuntimePlanNode {
                kind: RuntimePlanNodeKind::Case(case),
                ..
            }) => Some(GtkCaseBranch {
                case: case_ref,
                pattern: branch.pattern,
                body: self.child_group(
                    case_ref,
                    GtkChildGroupKind::CaseBody,
                    &case.children,
                    errors,
                ),
            }),
            Some(node) => {
                errors.push(GtkBridgeLoweringError::UnexpectedMatchCaseKind {
                    match_node: match_ref.plan,
                    case: branch.case.plan,
                    found: runtime_kind_tag(&node.kind),
                });
                None
            }
            None => {
                errors.push(GtkBridgeLoweringError::MissingRuntimeNode {
                    node: branch.case.plan,
                });
                None
            }
        }
    }

    fn build_empty_branch(
        &self,
        each_ref: GtkBridgeNodeRef,
        empty_ref: RuntimeNodeRef,
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) -> Option<GtkEmptyBranch> {
        self.validate_direct_child(each_ref, empty_ref.into(), errors);
        let empty_node_ref = GtkBridgeNodeRef::from(empty_ref);
        let empty_node = self.assembly.node(empty_ref.plan);
        match empty_node {
            Some(RuntimePlanNode {
                kind: RuntimePlanNodeKind::Empty(empty),
                ..
            }) => Some(GtkEmptyBranch {
                empty: empty_node_ref,
                body: self.child_group(
                    empty_node_ref,
                    GtkChildGroupKind::EachEmptyBranch,
                    &empty.children,
                    errors,
                ),
            }),
            Some(node) => {
                errors.push(GtkBridgeLoweringError::UnexpectedEachEmptyBranchKind {
                    each: each_ref.plan,
                    empty: empty_ref.plan,
                    found: runtime_kind_tag(&node.kind),
                });
                None
            }
            None => {
                errors.push(GtkBridgeLoweringError::MissingRuntimeNode {
                    node: empty_ref.plan,
                });
                None
            }
        }
    }

    fn child_group(
        &self,
        owner: GtkBridgeNodeRef,
        kind: GtkChildGroupKind,
        children: &[RuntimeChildOp],
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) -> GtkChildGroup {
        let mut roots = Vec::with_capacity(children.len());
        for child in children.iter().copied() {
            let child_ref = GtkBridgeNodeRef::from(child.child());
            self.validate_direct_child(owner, child_ref, errors);
            roots.push(child_ref);
        }
        GtkChildGroup {
            owner,
            kind,
            roots: roots.into_boxed_slice(),
        }
    }

    fn validate_direct_child(
        &self,
        parent: GtkBridgeNodeRef,
        child: GtkBridgeNodeRef,
        errors: &mut Vec<GtkBridgeLoweringError>,
    ) {
        match self.assembly.node(child.plan) {
            Some(node) => {
                let recorded = node.parent.map(GtkBridgeNodeRef::from);
                if recorded != Some(parent) {
                    errors.push(GtkBridgeLoweringError::ChildParentMismatch {
                        parent: parent.plan,
                        child: child.plan,
                        recorded: recorded.map(|node| node.plan),
                    });
                }
            }
            None => errors.push(GtkBridgeLoweringError::MissingRuntimeNode { node: child.plan }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkBridgeGraph {
    assembly: WidgetRuntimeAssembly,
    root: GtkBridgeNodeRef,
    nodes: Box<[GtkBridgeNode]>,
}

impl GtkBridgeGraph {
    pub fn assembly(&self) -> &WidgetRuntimeAssembly {
        &self.assembly
    }

    pub fn graph(&self) -> &SignalGraph {
        self.assembly.graph()
    }

    pub const fn root(&self) -> GtkBridgeNodeRef {
        self.root
    }

    pub fn root_node(&self) -> &GtkBridgeNode {
        self.node(self.root.plan)
            .expect("bridge graph root should always be materialized")
    }

    pub fn nodes(&self) -> &[GtkBridgeNode] {
        &self.nodes
    }

    pub fn node(&self, plan: PlanNodeId) -> Option<&GtkBridgeNode> {
        self.nodes.get(plan.index())
    }

    pub fn node_for_owner(&self, owner: OwnerHandle) -> Option<&GtkBridgeNode> {
        self.plan_for_owner(owner).and_then(|plan| self.node(plan))
    }

    pub fn node_ref(&self, plan: PlanNodeId) -> Option<GtkBridgeNodeRef> {
        self.assembly.node_ref(plan).map(GtkBridgeNodeRef::from)
    }

    pub fn plan_for_owner(&self, owner: OwnerHandle) -> Option<PlanNodeId> {
        self.assembly.plan_for_owner(owner)
    }

    pub fn show_transition(
        &self,
        node: GtkBridgeNodeRef,
        previous: GtkShowState,
        when: bool,
        keep_mounted: bool,
    ) -> Result<GtkShowTransition, GtkBridgeExecutionError> {
        let bridge_node = self.expect_node(node, PlanNodeTag::Show)?;
        let GtkBridgeNodeKind::Show(show) = &bridge_node.kind else {
            unreachable!();
        };
        let next = show.target_state(when, keep_mounted);
        let edits = match (previous, next) {
            (GtkShowState::Unmounted, GtkShowState::MountedVisible) => {
                vec![GtkChildGroupEdit::Mount {
                    group: show.body.clone(),
                }]
            }
            (GtkShowState::Unmounted, GtkShowState::MountedHidden) => vec![
                GtkChildGroupEdit::Mount {
                    group: show.body.clone(),
                },
                GtkChildGroupEdit::SetVisibility {
                    group: show.body.clone(),
                    visible: false,
                },
            ],
            (GtkShowState::MountedVisible, GtkShowState::Unmounted)
            | (GtkShowState::MountedHidden, GtkShowState::Unmounted) => {
                vec![GtkChildGroupEdit::Unmount {
                    group: show.body.clone(),
                }]
            }
            (GtkShowState::MountedVisible, GtkShowState::MountedHidden) => {
                vec![GtkChildGroupEdit::SetVisibility {
                    group: show.body.clone(),
                    visible: false,
                }]
            }
            (GtkShowState::MountedHidden, GtkShowState::MountedVisible) => {
                vec![GtkChildGroupEdit::SetVisibility {
                    group: show.body.clone(),
                    visible: true,
                }]
            }
            _ => Vec::new(),
        };
        Ok(GtkShowTransition {
            previous,
            next,
            edits: edits.into_boxed_slice(),
        })
    }

    pub fn match_transition(
        &self,
        node: GtkBridgeNodeRef,
        previous_case: Option<usize>,
        next_case: usize,
    ) -> Result<GtkMatchTransition, GtkBridgeExecutionError> {
        let bridge_node = self.expect_node(node, PlanNodeTag::Match)?;
        let GtkBridgeNodeKind::Match(match_node) = &bridge_node.kind else {
            unreachable!();
        };
        let next = match_node.case(next_case, node)?;
        let previous = previous_case
            .map(|index| match_node.case(index, node))
            .transpose()?;
        let edits = match &previous {
            Some(previous) if previous.case != next.case => vec![
                GtkChildGroupEdit::Unmount {
                    group: previous.body.clone(),
                },
                GtkChildGroupEdit::Mount {
                    group: next.body.clone(),
                },
            ],
            Some(_) => Vec::new(),
            None => vec![GtkChildGroupEdit::Mount {
                group: next.body.clone(),
            }],
        };
        Ok(GtkMatchTransition {
            previous,
            next,
            edits: edits.into_boxed_slice(),
        })
    }

    pub fn each_positional_transition(
        &self,
        node: GtkBridgeNodeRef,
        previous_len: usize,
        next_len: usize,
    ) -> Result<GtkEachTransition, GtkBridgeExecutionError> {
        let bridge_node = self.expect_node(node, PlanNodeTag::Each)?;
        let GtkBridgeNodeKind::Each(each) = &bridge_node.kind else {
            unreachable!();
        };
        let found = GtkRepeatedPolicyKind::of(&each.child_policy);
        if found != GtkRepeatedPolicyKind::Positional {
            return Err(GtkBridgeExecutionError::UnexpectedEachPolicy {
                node,
                expected: GtkRepeatedPolicyKind::Positional,
                found,
            });
        }

        let mut edits = Vec::new();
        if previous_len == 0 && next_len > 0 {
            if let Some(empty_branch) = &each.empty_branch {
                edits.push(GtkEachEdit::Group(GtkChildGroupEdit::Unmount {
                    group: empty_branch.body.clone(),
                }));
            }
        }

        if next_len < previous_len {
            for index in (next_len..previous_len).rev() {
                edits.push(GtkEachEdit::Item(GtkRepeatedChildEdit::Remove {
                    item: GtkRepeatedChildInstance::positional(each.item_template.clone(), index),
                    from: index,
                }));
            }
        } else {
            for index in previous_len..next_len {
                edits.push(GtkEachEdit::Item(GtkRepeatedChildEdit::Insert {
                    item: GtkRepeatedChildInstance::positional(each.item_template.clone(), index),
                    to: index,
                }));
            }
        }

        if previous_len > 0 && next_len == 0 {
            if let Some(empty_branch) = &each.empty_branch {
                edits.push(GtkEachEdit::Group(GtkChildGroupEdit::Mount {
                    group: empty_branch.body.clone(),
                }));
            }
        }

        Ok(GtkEachTransition {
            edits: edits.into_boxed_slice(),
        })
    }

    pub fn each_keyed_transition(
        &self,
        node: GtkBridgeNodeRef,
        previous: &[GtkCollectionKey],
        next: &[GtkCollectionKey],
    ) -> Result<GtkEachTransition, GtkBridgeExecutionError> {
        let bridge_node = self.expect_node(node, PlanNodeTag::Each)?;
        let GtkBridgeNodeKind::Each(each) = &bridge_node.kind else {
            unreachable!();
        };
        let found = GtkRepeatedPolicyKind::of(&each.child_policy);
        if found != GtkRepeatedPolicyKind::Keyed {
            return Err(GtkBridgeExecutionError::UnexpectedEachPolicy {
                node,
                expected: GtkRepeatedPolicyKind::Keyed,
                found,
            });
        }

        validate_unique_collection_keys(node, GtkCollectionStateSide::Previous, previous)?;
        validate_unique_collection_keys(node, GtkCollectionStateSide::Next, next)?;

        let mut edits = Vec::new();
        if previous.is_empty() && !next.is_empty() {
            if let Some(empty_branch) = &each.empty_branch {
                edits.push(GtkEachEdit::Group(GtkChildGroupEdit::Unmount {
                    group: empty_branch.body.clone(),
                }));
            }
        }

        let mut current = previous.to_vec();
        let next_keys = next.iter().cloned().collect::<BTreeSet<_>>();
        for index in (0..current.len()).rev() {
            if !next_keys.contains(&current[index]) {
                let key = current.remove(index);
                edits.push(GtkEachEdit::Item(GtkRepeatedChildEdit::Remove {
                    item: GtkRepeatedChildInstance::keyed(each.item_template.clone(), key),
                    from: index,
                }));
            }
        }

        for (to, key) in next.iter().cloned().enumerate() {
            if current.get(to) == Some(&key) {
                continue;
            }
            if let Some(from) = current
                .iter()
                .enumerate()
                .skip(to + 1)
                .find_map(|(from, candidate)| (*candidate == key).then_some(from))
            {
                current.remove(from);
                current.insert(to, key.clone());
                edits.push(GtkEachEdit::Item(GtkRepeatedChildEdit::Move {
                    item: GtkRepeatedChildInstance::keyed(each.item_template.clone(), key),
                    from,
                    to,
                }));
            } else {
                current.insert(to, key.clone());
                edits.push(GtkEachEdit::Item(GtkRepeatedChildEdit::Insert {
                    item: GtkRepeatedChildInstance::keyed(each.item_template.clone(), key),
                    to,
                }));
            }
        }

        debug_assert_eq!(current, next);

        if !previous.is_empty() && next.is_empty() {
            if let Some(empty_branch) = &each.empty_branch {
                edits.push(GtkEachEdit::Group(GtkChildGroupEdit::Mount {
                    group: empty_branch.body.clone(),
                }));
            }
        }

        Ok(GtkEachTransition {
            edits: edits.into_boxed_slice(),
        })
    }

    fn expect_node(
        &self,
        node: GtkBridgeNodeRef,
        expected: PlanNodeTag,
    ) -> Result<&GtkBridgeNode, GtkBridgeExecutionError> {
        let found = self
            .node(node.plan)
            .ok_or(GtkBridgeExecutionError::MissingNode { node })?;
        if found.owner != node.owner {
            return Err(GtkBridgeExecutionError::NodeOwnerMismatch {
                node,
                recorded: found.owner,
            });
        }
        let found_tag = found.kind.tag();
        if found_tag != expected {
            return Err(GtkBridgeExecutionError::UnexpectedNodeKind {
                node,
                expected,
                found: found_tag,
            });
        }
        Ok(found)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkBridgeNodeRef {
    pub plan: PlanNodeId,
    pub owner: OwnerHandle,
}

impl From<RuntimeNodeRef> for GtkBridgeNodeRef {
    fn from(value: RuntimeNodeRef) -> Self {
        Self {
            plan: value.plan,
            owner: value.owner,
        }
    }
}

impl fmt::Display for GtkBridgeNodeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@owner:{}", self.plan, self.owner.as_raw())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkBridgeNode {
    pub plan: PlanNodeId,
    pub stable_id: StableNodeId,
    pub span: SourceSpan,
    pub owner: OwnerHandle,
    pub parent: Option<GtkBridgeNodeRef>,
    pub kind: GtkBridgeNodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkBridgeNodeKind {
    Widget(GtkWidgetNode),
    Show(GtkShowNode),
    Each(GtkEachNode),
    Empty(GtkEmptyNode),
    Match(GtkMatchNode),
    Case(GtkCaseNode),
    Fragment(GtkFragmentNode),
    With(GtkWithNode),
}

impl GtkBridgeNodeKind {
    pub const fn tag(&self) -> PlanNodeTag {
        match self {
            Self::Widget(_) => PlanNodeTag::Widget,
            Self::Show(_) => PlanNodeTag::Show,
            Self::Each(_) => PlanNodeTag::Each,
            Self::Empty(_) => PlanNodeTag::Empty,
            Self::Match(_) => PlanNodeTag::Match,
            Self::Case(_) => PlanNodeTag::Case,
            Self::Fragment(_) => PlanNodeTag::Fragment,
            Self::With(_) => PlanNodeTag::With,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkWidgetNode {
    pub widget: NamePath,
    pub properties: Box<[RuntimePropertyBinding]>,
    pub event_hooks: Box<[RuntimeEventBinding]>,
    pub default_children: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkShowNode {
    pub when: RuntimeExprInput,
    pub mount: RuntimeShowMountPolicy,
    pub body: GtkChildGroup,
}

impl GtkShowNode {
    pub const fn target_state(&self, when: bool, keep_mounted: bool) -> GtkShowState {
        match (self.should_keep_mounted(keep_mounted), when) {
            (false, false) => GtkShowState::Unmounted,
            (false, true) => GtkShowState::MountedVisible,
            (true, false) => GtkShowState::MountedHidden,
            (true, true) => GtkShowState::MountedVisible,
        }
    }

    const fn should_keep_mounted(&self, keep_mounted: bool) -> bool {
        match self.mount {
            RuntimeShowMountPolicy::UnmountWhenHidden => false,
            RuntimeShowMountPolicy::KeepMounted { .. } => keep_mounted,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkShowState {
    Unmounted,
    MountedHidden,
    MountedVisible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkEachNode {
    pub collection: RuntimeExprInput,
    pub key_input: Option<RuntimeExprInput>,
    pub binding: BindingId,
    pub child_policy: RepeatedChildPolicy,
    pub item_template: GtkChildGroup,
    pub empty_branch: Option<GtkEmptyBranch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkEmptyBranch {
    pub empty: GtkBridgeNodeRef,
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkEmptyNode {
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkMatchNode {
    pub scrutinee: RuntimeExprInput,
    pub cases: Box<[GtkCaseBranch]>,
}

impl GtkMatchNode {
    fn case(
        &self,
        index: usize,
        node: GtkBridgeNodeRef,
    ) -> Result<GtkCaseBranch, GtkBridgeExecutionError> {
        self.cases
            .get(index)
            .cloned()
            .ok_or(GtkBridgeExecutionError::MatchCaseOutOfRange {
                node,
                index,
                case_count: self.cases.len(),
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkCaseBranch {
    pub case: GtkBridgeNodeRef,
    pub pattern: PatternId,
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkCaseNode {
    pub pattern: PatternId,
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkFragmentNode {
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkWithNode {
    pub value: RuntimeExprInput,
    pub binding: BindingId,
    pub body: GtkChildGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkChildGroup {
    pub owner: GtkBridgeNodeRef,
    pub kind: GtkChildGroupKind,
    pub roots: Box<[GtkBridgeNodeRef]>,
}

impl GtkChildGroup {
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkChildGroupKind {
    WidgetDefault,
    ShowBody,
    EachItemTemplate,
    EachEmptyBranch,
    EmptyBody,
    CaseBody,
    FragmentBody,
    WithBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkShowTransition {
    pub previous: GtkShowState,
    pub next: GtkShowState,
    pub edits: Box<[GtkChildGroupEdit]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkMatchTransition {
    pub previous: Option<GtkCaseBranch>,
    pub next: GtkCaseBranch,
    pub edits: Box<[GtkChildGroupEdit]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkEachTransition {
    pub edits: Box<[GtkEachEdit]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkChildGroupEdit {
    Mount { group: GtkChildGroup },
    Unmount { group: GtkChildGroup },
    SetVisibility { group: GtkChildGroup, visible: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkEachEdit {
    Group(GtkChildGroupEdit),
    Item(GtkRepeatedChildEdit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkRepeatedChildEdit {
    Insert {
        item: GtkRepeatedChildInstance,
        to: usize,
    },
    Move {
        item: GtkRepeatedChildInstance,
        from: usize,
        to: usize,
    },
    Remove {
        item: GtkRepeatedChildInstance,
        from: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkRepeatedChildInstance {
    pub template: GtkChildGroup,
    pub identity: GtkRepeatedChildIdentity,
}

impl GtkRepeatedChildInstance {
    pub fn positional(template: GtkChildGroup, index: usize) -> Self {
        Self {
            template,
            identity: GtkRepeatedChildIdentity::Positional(index),
        }
    }

    pub fn keyed(template: GtkChildGroup, key: GtkCollectionKey) -> Self {
        Self {
            template,
            identity: GtkRepeatedChildIdentity::Keyed(key),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GtkRepeatedChildIdentity {
    Positional(usize),
    Keyed(GtkCollectionKey),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkCollectionKey(Box<str>);

impl GtkCollectionKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().into_boxed_str())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GtkCollectionKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for GtkCollectionKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for GtkCollectionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkRepeatedPolicyKind {
    Positional,
    Keyed,
}

impl GtkRepeatedPolicyKind {
    pub const fn of(policy: &RepeatedChildPolicy) -> Self {
        match policy {
            RepeatedChildPolicy::Positional { .. } => Self::Positional,
            RepeatedChildPolicy::Keyed { .. } => Self::Keyed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkCollectionStateSide {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkBridgeExecutionError {
    MissingNode {
        node: GtkBridgeNodeRef,
    },
    NodeOwnerMismatch {
        node: GtkBridgeNodeRef,
        recorded: OwnerHandle,
    },
    UnexpectedNodeKind {
        node: GtkBridgeNodeRef,
        expected: PlanNodeTag,
        found: PlanNodeTag,
    },
    UnexpectedEachPolicy {
        node: GtkBridgeNodeRef,
        expected: GtkRepeatedPolicyKind,
        found: GtkRepeatedPolicyKind,
    },
    MatchCaseOutOfRange {
        node: GtkBridgeNodeRef,
        index: usize,
        case_count: usize,
    },
    DuplicateCollectionKey {
        node: GtkBridgeNodeRef,
        side: GtkCollectionStateSide,
        key: GtkCollectionKey,
    },
}

impl fmt::Display for GtkBridgeExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNode { node } => write!(f, "GTK bridge node {node} is missing"),
            Self::NodeOwnerMismatch { node, recorded } => write!(
                f,
                "GTK bridge node {node} expected owner {}, found {}",
                node.owner.as_raw(),
                recorded.as_raw()
            ),
            Self::UnexpectedNodeKind {
                node,
                expected,
                found,
            } => write!(
                f,
                "GTK bridge node {node} expected {expected:?}, found {found:?}"
            ),
            Self::UnexpectedEachPolicy {
                node,
                expected,
                found,
            } => write!(
                f,
                "GTK bridge each node {node} expected {expected:?} policy, found {found:?}"
            ),
            Self::MatchCaseOutOfRange {
                node,
                index,
                case_count,
            } => write!(
                f,
                "GTK bridge match node {node} requested case {index}, but only {case_count} case(s) exist"
            ),
            Self::DuplicateCollectionKey { node, side, key } => write!(
                f,
                "GTK bridge each node {node} saw duplicate key `{key}` in the {side:?} collection state"
            ),
        }
    }
}

impl Error for GtkBridgeExecutionError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkBridgeLoweringErrors {
    errors: Box<[GtkBridgeLoweringError]>,
}

impl GtkBridgeLoweringErrors {
    pub fn new(errors: Vec<GtkBridgeLoweringError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[GtkBridgeLoweringError] {
        &self.errors
    }
}

impl From<WidgetRuntimeAdapterErrors> for GtkBridgeLoweringErrors {
    fn from(value: WidgetRuntimeAdapterErrors) -> Self {
        Self::new(
            value
                .errors()
                .iter()
                .cloned()
                .map(GtkBridgeLoweringError::RuntimeAssembly)
                .collect(),
        )
    }
}

impl fmt::Display for GtkBridgeLoweringErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "failed to lower runtime assembly into a GTK bridge graph:"
        )?;
        for error in &self.errors {
            writeln!(f, "- {error}")?;
        }
        Ok(())
    }
}

impl Error for GtkBridgeLoweringErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkBridgeLoweringError {
    RuntimeAssembly(WidgetRuntimeAdapterError),
    MissingRuntimeNode {
        node: PlanNodeId,
    },
    RootHasParent {
        root: PlanNodeId,
        parent: PlanNodeId,
    },
    AdditionalRoot {
        expected: PlanNodeId,
        additional: PlanNodeId,
    },
    ChildParentMismatch {
        parent: PlanNodeId,
        child: PlanNodeId,
        recorded: Option<PlanNodeId>,
    },
    UnexpectedEachEmptyBranchKind {
        each: PlanNodeId,
        empty: PlanNodeId,
        found: PlanNodeTag,
    },
    UnexpectedMatchCaseKind {
        match_node: PlanNodeId,
        case: PlanNodeId,
        found: PlanNodeTag,
    },
}

impl fmt::Display for GtkBridgeLoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeAssembly(error) => write!(f, "runtime assembly failed: {error}"),
            Self::MissingRuntimeNode { node } => {
                write!(f, "runtime assembly node {node} is missing")
            }
            Self::RootHasParent { root, parent } => write!(
                f,
                "runtime assembly root {root} unexpectedly records parent {parent}"
            ),
            Self::AdditionalRoot {
                expected,
                additional,
            } => write!(
                f,
                "runtime assembly expected root {expected}, but node {additional} is also parentless"
            ),
            Self::ChildParentMismatch {
                parent,
                child,
                recorded,
            } => match recorded {
                Some(recorded) => write!(
                    f,
                    "runtime assembly child {child} is owned by {recorded}, not expected parent {parent}"
                ),
                None => write!(
                    f,
                    "runtime assembly child {child} is missing a parent, expected {parent}"
                ),
            },
            Self::UnexpectedEachEmptyBranchKind { each, empty, found } => write!(
                f,
                "runtime assembly each node {each} expected empty branch {empty} to be Empty, found {found:?}"
            ),
            Self::UnexpectedMatchCaseKind {
                match_node,
                case,
                found,
            } => write!(
                f,
                "runtime assembly match node {match_node} expected case {case} to be Case, found {found:?}"
            ),
        }
    }
}

impl Error for GtkBridgeLoweringError {}

fn validate_unique_collection_keys(
    node: GtkBridgeNodeRef,
    side: GtkCollectionStateSide,
    keys: &[GtkCollectionKey],
) -> Result<(), GtkBridgeExecutionError> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key.clone()) {
            return Err(GtkBridgeExecutionError::DuplicateCollectionKey {
                node,
                side,
                key: key.clone(),
            });
        }
    }
    Ok(())
}

fn runtime_kind_tag(kind: &RuntimePlanNodeKind) -> PlanNodeTag {
    match kind {
        RuntimePlanNodeKind::Widget(_) => PlanNodeTag::Widget,
        RuntimePlanNodeKind::Show(_) => PlanNodeTag::Show,
        RuntimePlanNodeKind::Each(_) => PlanNodeTag::Each,
        RuntimePlanNodeKind::Empty(_) => PlanNodeTag::Empty,
        RuntimePlanNodeKind::Match(_) => PlanNodeTag::Match,
        RuntimePlanNodeKind::Case(_) => PlanNodeTag::Case,
        RuntimePlanNodeKind::Fragment(_) => PlanNodeTag::Fragment,
        RuntimePlanNodeKind::With(_) => PlanNodeTag::With,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module};
    use aivi_syntax::parse_module;

    use super::*;
    use crate::lower_markup_expr;

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

    fn key(text: &str) -> GtkCollectionKey {
        GtkCollectionKey::from(text)
    }

    fn control_fixture_graph() -> GtkBridgeGraph {
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
        lower_widget_bridge(&plan).expect("fixture should lower into a GTK bridge graph")
    }

    fn positional_each_graph() -> (GtkBridgeGraph, GtkBridgeNodeRef) {
        let hir = lower_text(
            "positional-each.aivi",
            r#"
val items = [1]
val view =
    <each of={items} as={item}>
        <Label text="Row" />
    </each>
"#,
        );
        assert!(
            !hir.has_errors(),
            "positional each fixture should lower cleanly: {:?}",
            hir.diagnostics()
        );
        let module = hir.module();
        let value = find_value_item(module, "view");
        let plan = lower_markup_expr(module, value.body).expect("markup should lower");
        let graph = lower_widget_bridge(&plan).expect("GTK bridge graph should build");
        let root = graph.root();
        (graph, root)
    }

    fn show_match_each_refs(
        graph: &GtkBridgeGraph,
    ) -> (GtkBridgeNodeRef, GtkBridgeNodeRef, GtkBridgeNodeRef) {
        let root = graph.root_node();
        let GtkBridgeNodeKind::Fragment(fragment) = &root.kind else {
            panic!("expected fragment root, found {:?}", root.kind.tag());
        };
        let show_ref = fragment.body.roots[1];
        let show = graph.node(show_ref.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_node) = &show.kind else {
            panic!("expected show node, found {:?}", show.kind.tag());
        };
        let with_ref = show_node.body.roots[0];
        let with = graph.node(with_ref.plan).expect("with node should exist");
        let GtkBridgeNodeKind::With(with_node) = &with.kind else {
            panic!("expected with node, found {:?}", with.kind.tag());
        };
        let match_ref = with_node.body.roots[0];
        let match_node = graph.node(match_ref.plan).expect("match node should exist");
        let GtkBridgeNodeKind::Match(match_node) = &match_node.kind else {
            panic!("expected match node, found {:?}", match_node.kind.tag());
        };
        let each_ref = match_node.cases[1].body.roots[0];
        (show_ref, match_ref, each_ref)
    }

    #[test]
    fn lowers_runtime_assembly_into_gtk_bridge_graph_with_body_groups() {
        let graph = control_fixture_graph();
        assert_eq!(graph.graph().owner_count(), graph.nodes().len());

        let root = graph.root_node();
        let GtkBridgeNodeKind::Fragment(fragment) = &root.kind else {
            panic!("expected fragment root, found {:?}", root.kind.tag());
        };
        assert_eq!(graph.plan_for_owner(root.owner), Some(root.plan));
        assert_eq!(fragment.body.kind, GtkChildGroupKind::FragmentBody);
        assert_eq!(fragment.body.len(), 2);

        let show_ref = fragment.body.roots[1];
        let show = graph.node(show_ref.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_node) = &show.kind else {
            panic!("expected show node, found {:?}", show.kind.tag());
        };
        assert_eq!(show_node.body.kind, GtkChildGroupKind::ShowBody);
        assert_eq!(show_node.body.len(), 1);

        let with_ref = show_node.body.roots[0];
        let with = graph.node(with_ref.plan).expect("with node should exist");
        let GtkBridgeNodeKind::With(with_node) = &with.kind else {
            panic!("expected with node, found {:?}", with.kind.tag());
        };
        assert_eq!(with_node.body.kind, GtkChildGroupKind::WithBody);

        let match_ref = with_node.body.roots[0];
        let match_node = graph.node(match_ref.plan).expect("match node should exist");
        let GtkBridgeNodeKind::Match(match_node) = &match_node.kind else {
            panic!("expected match node, found {:?}", match_node.kind.tag());
        };
        assert_eq!(match_node.cases.len(), 3);
        assert_eq!(match_node.cases[1].body.kind, GtkChildGroupKind::CaseBody);

        let each_ref = match_node.cases[1].body.roots[0];
        let each = graph.node(each_ref.plan).expect("each node should exist");
        let GtkBridgeNodeKind::Each(each_node) = &each.kind else {
            panic!("expected each node, found {:?}", each.kind.tag());
        };
        assert_eq!(
            each_node.item_template.kind,
            GtkChildGroupKind::EachItemTemplate
        );
        let empty_branch = each_node
            .empty_branch
            .as_ref()
            .expect("each node should keep explicit empty branch semantics");
        assert_eq!(empty_branch.body.kind, GtkChildGroupKind::EachEmptyBranch);
    }

    #[test]
    fn show_transition_keeps_mounted_children_without_unmounting() {
        let graph = control_fixture_graph();
        let (show_ref, _, _) = show_match_each_refs(&graph);
        let show = graph.node(show_ref.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_node) = &show.kind else {
            panic!("expected show node, found {:?}", show.kind.tag());
        };

        let hidden = graph
            .show_transition(show_ref, GtkShowState::Unmounted, false, true)
            .expect("keepMounted show should transition");
        assert_eq!(hidden.next, GtkShowState::MountedHidden);
        assert_eq!(
            hidden.edits.as_ref(),
            &[
                GtkChildGroupEdit::Mount {
                    group: show_node.body.clone(),
                },
                GtkChildGroupEdit::SetVisibility {
                    group: show_node.body.clone(),
                    visible: false,
                },
            ]
        );

        let visible = graph
            .show_transition(show_ref, GtkShowState::MountedHidden, true, true)
            .expect("mounted show should transition back to visible");
        assert_eq!(visible.next, GtkShowState::MountedVisible);
        assert_eq!(
            visible.edits.as_ref(),
            &[GtkChildGroupEdit::SetVisibility {
                group: show_node.body.clone(),
                visible: true,
            }]
        );
    }

    #[test]
    fn show_transition_respects_runtime_keep_mounted_decision() {
        let hir = lower_text(
            "show-keep-mounted.aivi",
            r#"
val view =
    <show when={True} keepMounted={False}>
        <Label text="Ready" />
    </show>
"#,
        );
        assert!(
            !hir.has_errors(),
            "show fixture should lower cleanly: {:?}",
            hir.diagnostics()
        );
        let module = hir.module();
        let value = find_value_item(module, "view");
        let plan = lower_markup_expr(module, value.body).expect("markup should lower");
        let graph = lower_widget_bridge(&plan).expect("GTK bridge graph should build");
        let show_ref = graph.root();

        let hidden = graph
            .show_transition(show_ref, GtkShowState::Unmounted, false, false)
            .expect("keepMounted=False should follow unmount semantics");
        assert_eq!(hidden.next, GtkShowState::Unmounted);
        assert!(hidden.edits.is_empty());

        let visible = graph
            .show_transition(show_ref, GtkShowState::Unmounted, true, false)
            .expect("show should still mount when visible");
        assert_eq!(visible.next, GtkShowState::MountedVisible);
        assert_eq!(visible.edits.len(), 1);
    }

    #[test]
    fn each_keyed_transition_emits_localized_child_edits_and_empty_branch_changes() {
        let graph = control_fixture_graph();
        let (_, _, each_ref) = show_match_each_refs(&graph);
        let each = graph.node(each_ref.plan).expect("each node should exist");
        let GtkBridgeNodeKind::Each(each_node) = &each.kind else {
            panic!("expected each node, found {:?}", each.kind.tag());
        };
        let empty_branch = each_node
            .empty_branch
            .as_ref()
            .expect("keyed fixture should keep an empty branch")
            .body
            .clone();

        let initial = graph
            .each_keyed_transition(each_ref, &[], &[key("alpha")])
            .expect("keyed each should accept localized insertion");
        assert_eq!(
            initial.edits.as_ref(),
            &[
                GtkEachEdit::Group(GtkChildGroupEdit::Unmount {
                    group: empty_branch.clone(),
                }),
                GtkEachEdit::Item(GtkRepeatedChildEdit::Insert {
                    item: GtkRepeatedChildInstance::keyed(
                        each_node.item_template.clone(),
                        key("alpha"),
                    ),
                    to: 0,
                }),
            ]
        );

        let reorder = graph
            .each_keyed_transition(
                each_ref,
                &[key("a"), key("b"), key("c")],
                &[key("c"), key("b"), key("a")],
            )
            .expect("keyed each should reuse keyed children through moves");
        assert_eq!(
            reorder.edits.as_ref(),
            &[
                GtkEachEdit::Item(GtkRepeatedChildEdit::Move {
                    item: GtkRepeatedChildInstance::keyed(
                        each_node.item_template.clone(),
                        key("c"),
                    ),
                    from: 2,
                    to: 0,
                }),
                GtkEachEdit::Item(GtkRepeatedChildEdit::Move {
                    item: GtkRepeatedChildInstance::keyed(
                        each_node.item_template.clone(),
                        key("b"),
                    ),
                    from: 2,
                    to: 1,
                }),
            ]
        );
    }

    #[test]
    fn each_keyed_transition_rejects_duplicate_keys() {
        let graph = control_fixture_graph();
        let (_, _, each_ref) = show_match_each_refs(&graph);
        let error = graph
            .each_keyed_transition(each_ref, &[key("dup"), key("dup")], &[])
            .expect_err("duplicate keyed children must be rejected");
        assert_eq!(
            error,
            GtkBridgeExecutionError::DuplicateCollectionKey {
                node: each_ref,
                side: GtkCollectionStateSide::Previous,
                key: key("dup"),
            }
        );
    }

    #[test]
    fn match_transition_swaps_case_groups_directly() {
        let graph = control_fixture_graph();
        let (_, match_ref, _) = show_match_each_refs(&graph);
        let match_node = graph.node(match_ref.plan).expect("match node should exist");
        let GtkBridgeNodeKind::Match(match_node) = &match_node.kind else {
            panic!("expected match node, found {:?}", match_node.kind.tag());
        };

        let transition = graph
            .match_transition(match_ref, Some(0), 2)
            .expect("match transition should select concrete case groups");
        assert_eq!(
            transition.previous.as_ref().map(|case| case.case),
            Some(match_node.cases[0].case)
        );
        assert_eq!(transition.next.case, match_node.cases[2].case);
        assert_eq!(
            transition.edits.as_ref(),
            &[
                GtkChildGroupEdit::Unmount {
                    group: match_node.cases[0].body.clone(),
                },
                GtkChildGroupEdit::Mount {
                    group: match_node.cases[2].body.clone(),
                },
            ]
        );
    }

    #[test]
    fn positional_each_transition_resizes_without_keyed_moves() {
        let (graph, each_ref) = positional_each_graph();
        let each = graph.node(each_ref.plan).expect("each node should exist");
        let GtkBridgeNodeKind::Each(each_node) = &each.kind else {
            panic!("expected each node, found {:?}", each.kind.tag());
        };

        let transition = graph
            .each_positional_transition(each_ref, 1, 3)
            .expect("positional each should resize");
        assert_eq!(
            transition.edits.as_ref(),
            &[
                GtkEachEdit::Item(GtkRepeatedChildEdit::Insert {
                    item: GtkRepeatedChildInstance::positional(each_node.item_template.clone(), 1,),
                    to: 1,
                }),
                GtkEachEdit::Item(GtkRepeatedChildEdit::Insert {
                    item: GtkRepeatedChildInstance::positional(each_node.item_template.clone(), 2,),
                    to: 2,
                }),
            ]
        );
    }
}
