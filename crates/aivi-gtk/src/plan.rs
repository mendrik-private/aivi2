use std::{collections::HashMap, error::Error, fmt};

use aivi_base::SourceSpan;
use aivi_hir::{
    BindingId, ControlNodeId, ExprId, MarkupNodeId, Name, NamePath, NonEmpty, PatternId,
    TextLiteral,
};

/// One node in the lowered widget plan arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanNodeId(u32);

impl PlanNodeId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for PlanNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "plan-node:{}", self.0)
    }
}

/// Stable identity imported from HIR so later runtime layers can preserve widget/control identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StableNodeId {
    Markup(MarkupNodeId),
    Control(ControlNodeId),
}

const MAX_PLAN_NESTING_DEPTH: usize = 128;

/// Full lowered widget/control graph rooted at one markup expression.
///
/// Ownership model: the plan owns its node arena and refers back into HIR only through typed IDs.
/// Identity strategy: every plan node carries the originating HIR markup/control identity.
/// Span strategy: every node and attribute site stores the originating source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetPlan {
    root: PlanNodeId,
    nodes: Vec<PlanNode>,
}

impl WidgetPlan {
    pub fn new(root: PlanNodeId, nodes: Vec<PlanNode>) -> Self {
        Self { root, nodes }
    }

    pub const fn root(&self) -> PlanNodeId {
        self.root
    }

    pub fn node(&self, id: PlanNodeId) -> Option<&PlanNode> {
        self.nodes.get(id.index())
    }

    pub fn nodes(&self) -> &[PlanNode] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn validate(&self) -> Result<(), PlanValidationError> {
        if self.node(self.root).is_none() {
            return Err(PlanValidationError::MissingRoot(self.root));
        }

        let mut seen = HashMap::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            let plan_id = PlanNodeId::new(index as u32);
            if let Some(first) = seen.insert(node.stable_id, plan_id) {
                return Err(PlanValidationError::DuplicateStableIdentity {
                    stable_id: node.stable_id,
                    first,
                    duplicate: plan_id,
                });
            }

            match &node.kind {
                PlanNodeKind::Widget(widget) => {
                    self.validate_child_ops(plan_id, &widget.children)?;
                }
                PlanNodeKind::Group(group) => {
                    self.validate_child_ops(plan_id, &group.children)?;
                }
                PlanNodeKind::Show(show) => {
                    self.validate_child_ops(plan_id, &show.children)?;
                }
                PlanNodeKind::Each(each) => {
                    self.validate_child_ops(plan_id, &each.item_children)?;
                    if let Some(empty) = each.empty_branch {
                        let empty_node =
                            self.node(empty).ok_or(PlanValidationError::MissingBranch {
                                parent: plan_id,
                                branch: empty,
                            })?;
                        if empty_node.kind.tag() != PlanNodeTag::Empty {
                            return Err(PlanValidationError::UnexpectedBranchKind {
                                parent: plan_id,
                                branch: empty,
                                expected: PlanNodeTag::Empty,
                                found: empty_node.kind.tag(),
                            });
                        }
                    }
                }
                PlanNodeKind::Empty(empty) => {
                    self.validate_child_ops(plan_id, &empty.children)?;
                }
                PlanNodeKind::Match(match_node) => {
                    for case in match_node.cases.iter().copied() {
                        let case_node =
                            self.node(case).ok_or(PlanValidationError::MissingBranch {
                                parent: plan_id,
                                branch: case,
                            })?;
                        if case_node.kind.tag() != PlanNodeTag::Case {
                            return Err(PlanValidationError::UnexpectedBranchKind {
                                parent: plan_id,
                                branch: case,
                                expected: PlanNodeTag::Case,
                                found: case_node.kind.tag(),
                            });
                        }
                    }
                }
                PlanNodeKind::Case(case) => {
                    self.validate_child_ops(plan_id, &case.children)?;
                }
                PlanNodeKind::Fragment(fragment) => {
                    self.validate_child_ops(plan_id, &fragment.children)?;
                }
                PlanNodeKind::With(with_node) => {
                    self.validate_child_ops(plan_id, &with_node.children)?;
                }
            }
        }

        self.validate_nesting_depth(self.root, 0)?;

        Ok(())
    }

    fn validate_nesting_depth(
        &self,
        node_id: PlanNodeId,
        depth: usize,
    ) -> Result<(), PlanValidationError> {
        if depth > MAX_PLAN_NESTING_DEPTH {
            return Err(PlanValidationError::NestingTooDeep {
                max: MAX_PLAN_NESTING_DEPTH,
            });
        }
        let Some(node) = self.node(node_id) else {
            return Ok(());
        };
        let children: Vec<PlanNodeId> = match &node.kind {
            PlanNodeKind::Widget(widget) => {
                widget.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::Group(group) => {
                group.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::Show(show) => {
                show.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::Each(each) => {
                let mut ids: Vec<PlanNodeId> =
                    each.item_children.iter().map(|op| op.child()).collect();
                if let Some(empty) = each.empty_branch {
                    ids.push(empty);
                }
                ids
            }
            PlanNodeKind::Empty(empty) => {
                empty.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::Match(match_node) => {
                match_node.cases.iter().copied().collect()
            }
            PlanNodeKind::Case(case) => {
                case.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::Fragment(fragment) => {
                fragment.children.iter().map(|op| op.child()).collect()
            }
            PlanNodeKind::With(with_node) => {
                with_node.children.iter().map(|op| op.child()).collect()
            }
        };
        for child in children {
            self.validate_nesting_depth(child, depth + 1)?;
        }
        Ok(())
    }

    fn validate_child_ops(
        &self,
        parent: PlanNodeId,
        children: &[ChildOp],
    ) -> Result<(), PlanValidationError> {
        for child in children {
            let child_id = child.child();
            if self.node(child_id).is_none() {
                return Err(PlanValidationError::MissingChild {
                    parent,
                    child: child_id,
                });
            }
        }
        Ok(())
    }
}

/// One lowered widget or control node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanNode {
    pub stable_id: StableNodeId,
    pub span: SourceSpan,
    pub kind: PlanNodeKind,
}

/// Stable discriminant for plan validation and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanNodeTag {
    Widget,
    Group,
    Show,
    Each,
    Empty,
    Match,
    Case,
    Fragment,
    With,
}

/// One lowered widget or control shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanNodeKind {
    Widget(WidgetNode),
    Group(GroupNode),
    Show(ShowNode),
    Each(EachNode),
    Empty(EmptyNode),
    Match(MatchNode),
    Case(CaseNode),
    Fragment(FragmentNode),
    With(WithNode),
}

impl PlanNodeKind {
    pub const fn tag(&self) -> PlanNodeTag {
        match self {
            Self::Widget(_) => PlanNodeTag::Widget,
            Self::Group(_) => PlanNodeTag::Group,
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

/// One explicit child-management instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChildOp {
    Append(PlanNodeId),
}

impl ChildOp {
    pub const fn child(self) -> PlanNodeId {
        match self {
            Self::Append(child) => child,
        }
    }
}

/// One lowered widget creation site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetNode {
    pub widget: NamePath,
    pub properties: Vec<PropertyPlan>,
    pub event_hooks: Vec<EventHookPlan>,
    pub children: Vec<ChildOp>,
}

/// Explicit named child-group wrapper such as `<Paned.start>...</Paned.start>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupNode {
    pub widget: NamePath,
    pub group: Name,
    pub children: Vec<ChildOp>,
}

/// Property initializer or setter binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyPlan {
    Static(StaticPropertyPlan),
    Setter(SetterBindingPlan),
}

/// Static property initialization that requires no runtime subscription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticPropertyPlan {
    pub site: AttributeSite,
    pub name: Name,
    pub value: StaticPropertyValue,
}

/// Surface-stable static property payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaticPropertyValue {
    ImplicitTrue,
    Text(TextLiteral),
}

/// Dynamic property binding lowered to a future direct setter call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetterBindingPlan {
    pub site: AttributeSite,
    pub name: Name,
    pub source: SetterSource,
    pub update: SetterUpdateStrategy,
    pub teardown: SetterTeardown,
}

/// One value source for a direct setter binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetterSource {
    Expr(ExprId),
    InterpolatedText(TextLiteral),
}

/// Concrete update mode mandated by the RFC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetterUpdateStrategy {
    DirectSetter,
}

/// Teardown work required for one setter binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetterTeardown {
    CancelSubscription,
}

/// Explicit event hookup lowered from markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventHookPlan {
    pub site: AttributeSite,
    pub name: Name,
    pub handler: ExprId,
    pub hookup: EventHookStrategy,
    pub teardown: EventHookTeardown,
}

/// Concrete event hookup mode mandated by the RFC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventHookStrategy {
    DirectSignal,
}

/// Teardown work required for one event hookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventHookTeardown {
    DisconnectHandler,
}

/// Stable location for one lowered attribute site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSite {
    pub owner: StableNodeId,
    pub index: usize,
    pub span: SourceSpan,
}

/// Lowered `<show>` control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowNode {
    pub when: ExprId,
    pub mount: ShowMountPolicy,
    pub children: Vec<ChildOp>,
}

/// Presence policy preserved for show/hide lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShowMountPolicy {
    UnmountWhenHidden,
    KeepMounted { decision: ExprId },
}

/// Lowered `<each>` control with explicit localized child management policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EachNode {
    pub collection: ExprId,
    pub binding: BindingId,
    pub child_policy: RepeatedChildPolicy,
    pub item_children: Vec<ChildOp>,
    pub empty_branch: Option<PlanNodeId>,
}

/// Child-management strategy for repeated subtrees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepeatedChildPolicy {
    Positional {
        updates: ChildUpdateMode,
    },
    Keyed {
        key: ExprId,
        updates: ChildUpdateMode,
    },
}

/// Repeated-child update mode. Kept explicit so later runtime work cannot regress into VDOM diffing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChildUpdateMode {
    Localized,
}

/// Lowered `<empty>` branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyNode {
    pub children: Vec<ChildOp>,
}

/// Lowered `<match>` control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchNode {
    pub scrutinee: ExprId,
    pub cases: NonEmpty<PlanNodeId>,
}

/// Lowered `<case>` branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseNode {
    pub pattern: PatternId,
    pub children: Vec<ChildOp>,
}

/// Lowered `<fragment>` control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentNode {
    pub children: Vec<ChildOp>,
}

/// Lowered `<with>` control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithNode {
    pub value: ExprId,
    pub binding: BindingId,
    pub children: Vec<ChildOp>,
}

/// Validation failure for the lowered plan arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanValidationError {
    MissingRoot(PlanNodeId),
    MissingChild {
        parent: PlanNodeId,
        child: PlanNodeId,
    },
    MissingBranch {
        parent: PlanNodeId,
        branch: PlanNodeId,
    },
    UnexpectedBranchKind {
        parent: PlanNodeId,
        branch: PlanNodeId,
        expected: PlanNodeTag,
        found: PlanNodeTag,
    },
    DuplicateStableIdentity {
        stable_id: StableNodeId,
        first: PlanNodeId,
        duplicate: PlanNodeId,
    },
    NestingTooDeep {
        max: usize,
    },
}

impl fmt::Display for PlanValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRoot(root) => write!(f, "widget plan root {root} is missing"),
            Self::MissingChild { parent, child } => {
                write!(
                    f,
                    "widget plan node {parent} references missing child {child}"
                )
            }
            Self::MissingBranch { parent, branch } => {
                write!(
                    f,
                    "widget plan node {parent} references missing branch {branch}"
                )
            }
            Self::UnexpectedBranchKind {
                parent,
                branch,
                expected,
                found,
            } => write!(
                f,
                "widget plan node {parent} expected branch {branch} to be {expected:?}, found {found:?}"
            ),
            Self::DuplicateStableIdentity {
                stable_id,
                first,
                duplicate,
            } => write!(
                f,
                "stable node identity {stable_id:?} appears at both {first} and {duplicate}"
            ),
            Self::NestingTooDeep { max } => write!(
                f,
                "widget plan nesting depth exceeds the maximum of {max}"
            ),
        }
    }
}

impl Error for PlanValidationError {}

#[cfg(test)]
mod tests {
    use aivi_base::{FileId, SourceSpan, Span};
    use aivi_hir::{ControlNodeId, Name, NamePath, TextLiteral};

    use super::{
        ChildOp, EachNode, EmptyNode, GroupNode, PlanNode, PlanNodeId, PlanNodeKind,
        PlanValidationError, RepeatedChildPolicy, StableNodeId, StaticPropertyPlan,
        StaticPropertyValue, WidgetNode, WidgetPlan,
    };

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

    fn name(text: &str) -> Name {
        Name::new(text, span()).expect("name should be valid")
    }

    #[test]
    fn validate_rejects_missing_child_references() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![PlanNode {
                stable_id: StableNodeId::Markup(aivi_hir::MarkupNodeId::from_raw(0)),
                span: span(),
                kind: PlanNodeKind::Widget(WidgetNode {
                    widget: widget_name("Label"),
                    properties: vec![super::PropertyPlan::Static(StaticPropertyPlan {
                        site: super::AttributeSite {
                            owner: StableNodeId::Markup(aivi_hir::MarkupNodeId::from_raw(0)),
                            index: 0,
                            span: span(),
                        },
                        name: name("text"),
                        value: StaticPropertyValue::Text(empty_text()),
                    })],
                    event_hooks: Vec::new(),
                    children: vec![ChildOp::Append(PlanNodeId::new(1))],
                }),
            }],
        );

        assert_eq!(
            plan.validate(),
            Err(PlanValidationError::MissingChild {
                parent: PlanNodeId::new(0),
                child: PlanNodeId::new(1),
            })
        );
    }

    #[test]
    fn validate_requires_empty_branches_to_point_at_empty_nodes() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![
                PlanNode {
                    stable_id: StableNodeId::Control(ControlNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Each(EachNode {
                        collection: aivi_hir::ExprId::from_raw(0),
                        binding: aivi_hir::BindingId::from_raw(0),
                        child_policy: RepeatedChildPolicy::Positional {
                            updates: super::ChildUpdateMode::Localized,
                        },
                        item_children: Vec::new(),
                        empty_branch: Some(PlanNodeId::new(1)),
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Control(ControlNodeId::from_raw(1)),
                    span: span(),
                    kind: PlanNodeKind::Widget(WidgetNode {
                        widget: widget_name("Label"),
                        properties: Vec::new(),
                        event_hooks: Vec::new(),
                        children: Vec::new(),
                    }),
                },
            ],
        );

        assert!(matches!(
            plan.validate(),
            Err(PlanValidationError::UnexpectedBranchKind { .. })
        ));
    }

    #[test]
    fn validate_accepts_explicit_empty_branch_nodes() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![
                PlanNode {
                    stable_id: StableNodeId::Control(ControlNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Each(EachNode {
                        collection: aivi_hir::ExprId::from_raw(0),
                        binding: aivi_hir::BindingId::from_raw(0),
                        child_policy: RepeatedChildPolicy::Positional {
                            updates: super::ChildUpdateMode::Localized,
                        },
                        item_children: Vec::new(),
                        empty_branch: Some(PlanNodeId::new(1)),
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Control(ControlNodeId::from_raw(1)),
                    span: span(),
                    kind: PlanNodeKind::Empty(EmptyNode {
                        children: Vec::new(),
                    }),
                },
            ],
        );

        plan.validate().expect("empty branch should validate");
    }

    #[test]
    fn validate_accepts_group_nodes() {
        let plan = WidgetPlan::new(
            PlanNodeId::new(0),
            vec![
                PlanNode {
                    stable_id: StableNodeId::Markup(aivi_hir::MarkupNodeId::from_raw(0)),
                    span: span(),
                    kind: PlanNodeKind::Widget(WidgetNode {
                        widget: widget_name("Paned"),
                        properties: Vec::new(),
                        event_hooks: Vec::new(),
                        children: vec![ChildOp::Append(PlanNodeId::new(1))],
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Markup(aivi_hir::MarkupNodeId::from_raw(1)),
                    span: span(),
                    kind: PlanNodeKind::Group(GroupNode {
                        widget: widget_name("Paned"),
                        group: name("start"),
                        children: vec![ChildOp::Append(PlanNodeId::new(2))],
                    }),
                },
                PlanNode {
                    stable_id: StableNodeId::Markup(aivi_hir::MarkupNodeId::from_raw(2)),
                    span: span(),
                    kind: PlanNodeKind::Widget(WidgetNode {
                        widget: widget_name("Label"),
                        properties: Vec::new(),
                        event_hooks: Vec::new(),
                        children: Vec::new(),
                    }),
                },
            ],
        );

        plan.validate().expect("group wrappers should validate");
    }
}
