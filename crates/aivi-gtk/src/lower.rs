use std::{collections::HashMap, error::Error, fmt};

use aivi_hir::{
    BindingId, ControlNode, ControlNodeId, ExprId, ExprKind, MarkupAttribute, MarkupAttributeValue,
    MarkupNodeId, MarkupNodeKind, Module, Name, NamePath, NonEmpty, PatternId, TextLiteral,
    TextSegment,
};

use crate::plan::{
    AttributeSite, CaseNode, ChildOp, ChildUpdateMode, EachNode, EmptyNode, EventHookPlan,
    EventHookStrategy, EventHookTeardown, FragmentNode, GroupNode, MatchNode, PlanNode, PlanNodeId,
    PlanNodeKind, PropertyPlan, RepeatedChildPolicy, SetterBindingPlan, SetterSource,
    SetterTeardown, SetterUpdateStrategy, ShowMountPolicy, ShowNode, StableNodeId,
    StaticPropertyPlan, StaticPropertyValue, WidgetNode, WidgetPlan, WithNode,
};
use crate::schema::{lookup_widget_event, lookup_widget_schema};

/// Lowering options for the first GTK bridge slice.
///
/// The live GTK surface is driven by explicit widget schema metadata.
///
/// Callers may optionally keep event lowering inside a narrower attribute namespace, but schema
/// lookup remains the source of truth for whether a widget/attribute pair is a live GTK event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoweringOptions {
    event_attribute_prefix: Box<str>,
}

impl LoweringOptions {
    pub fn new(event_attribute_prefix: impl Into<String>) -> Self {
        Self {
            event_attribute_prefix: event_attribute_prefix.into().into_boxed_str(),
        }
    }

    pub fn event_attribute_prefix(&self) -> &str {
        &self.event_attribute_prefix
    }

    pub fn with_event_attribute_prefix(
        mut self,
        event_attribute_prefix: impl Into<String>,
    ) -> Self {
        self.event_attribute_prefix = event_attribute_prefix.into().into_boxed_str();
        self
    }

    fn lowers_as_event(&self, widget: &aivi_hir::NamePath, attribute: &MarkupAttribute) -> bool {
        matches!(attribute.value, MarkupAttributeValue::Expr(_))
            && attribute
                .name
                .text()
                .starts_with(self.event_attribute_prefix())
            && lookup_widget_event(widget, attribute.name.text()).is_some()
    }
}

impl Default for LoweringOptions {
    fn default() -> Self {
        Self::new("")
    }
}

/// Error reported while lowering validated-or-validatable HIR markup into the widget plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoweringError {
    ExpectedMarkupExpr(ExprId),
    MissingMarkupNode(MarkupNodeId),
    MissingControlNode(ControlNodeId),
    MissingExpr(ExprId),
    MissingBinding(BindingId),
    MissingPattern(PatternId),
    MissingLoweredMarkupChild {
        parent: StableNodeId,
        child: MarkupNodeId,
    },
    MissingLoweredControlBranch {
        parent: StableNodeId,
        branch: ControlNodeId,
    },
    InvalidPlan(crate::plan::PlanValidationError),
    UnexpectedRootChildGroup {
        group: String,
        span: aivi_base::SourceSpan,
    },
    ChildGroupHasAttributes {
        group: String,
        span: aivi_base::SourceSpan,
    },
    MisplacedChildGroup {
        parent: StableNodeId,
        group: String,
        span: aivi_base::SourceSpan,
    },
    MismatchedChildGroupOwner {
        parent_widget: String,
        group: String,
        span: aivi_base::SourceSpan,
    },
    UnknownWidgetChildGroup {
        widget: String,
        group: String,
        span: aivi_base::SourceSpan,
    },
    UnknownWidget {
        name: String,
        span: aivi_base::SourceSpan,
    },
}

impl fmt::Display for LoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedMarkupExpr(expr) => write!(f, "expression {expr:?} is not a markup root"),
            Self::MissingMarkupNode(node) => {
                write!(f, "markup node {node:?} was not found in the HIR module")
            }
            Self::MissingControlNode(node) => {
                write!(f, "control node {node:?} was not found in the HIR module")
            }
            Self::MissingExpr(expr) => {
                write!(f, "expression {expr:?} was not found in the HIR module")
            }
            Self::MissingBinding(binding) => {
                write!(f, "binding {binding:?} was not found in the HIR module")
            }
            Self::MissingPattern(pattern) => {
                write!(f, "pattern {pattern:?} was not found in the HIR module")
            }
            Self::MissingLoweredMarkupChild { parent, child } => write!(
                f,
                "lowered parent {parent:?} references markup child {child:?} that was not lowered"
            ),
            Self::MissingLoweredControlBranch { parent, branch } => write!(
                f,
                "lowered parent {parent:?} references control branch {branch:?} that was not lowered"
            ),
            Self::InvalidPlan(error) => write!(f, "lowered widget plan is invalid: {error}"),
            Self::UnexpectedRootChildGroup { group, .. } => write!(
                f,
                "markup root cannot be a named GTK child-group wrapper `{group}`"
            ),
            Self::ChildGroupHasAttributes { group, .. } => write!(
                f,
                "GTK child-group wrapper `{group}` cannot carry attributes"
            ),
            Self::MisplacedChildGroup { parent, group, .. } => write!(
                f,
                "lowered parent {parent:?} cannot contain GTK child-group wrapper `{group}` directly"
            ),
            Self::MismatchedChildGroupOwner {
                parent_widget,
                group,
                ..
            } => write!(
                f,
                "GTK child-group wrapper `{group}` must appear directly under `<{parent_widget}>`"
            ),
            Self::UnknownWidgetChildGroup { widget, group, .. } => write!(
                f,
                "GTK widget `{widget}` does not declare child group `{group}`"
            ),
            Self::UnknownWidget { name, .. } => {
                write!(f, "widget `{name}` is not known to the GTK schema registry")
            }
        }
    }
}

impl Error for LoweringError {}

/// Lower one HIR markup expression with the default lowering options.
pub fn lower_markup_expr(module: &Module, expr: ExprId) -> Result<WidgetPlan, LoweringError> {
    lower_markup_expr_with_options(module, expr, LoweringOptions::default())
}

/// Lower one HIR markup expression with explicit lowering options.
pub fn lower_markup_expr_with_options(
    module: &Module,
    expr: ExprId,
    options: LoweringOptions,
) -> Result<WidgetPlan, LoweringError> {
    let expr_node = module
        .exprs()
        .get(expr)
        .ok_or(LoweringError::MissingExpr(expr))?;
    let ExprKind::Markup(root) = expr_node.kind else {
        return Err(LoweringError::ExpectedMarkupExpr(expr));
    };
    lower_markup_root_with_options(module, root, options)
}

/// Lower one HIR markup root with the default lowering options.
pub fn lower_markup_root(module: &Module, root: MarkupNodeId) -> Result<WidgetPlan, LoweringError> {
    lower_markup_root_with_options(module, root, LoweringOptions::default())
}

/// Lower one HIR markup root into a typed widget plan.
pub fn lower_markup_root_with_options(
    module: &Module,
    root: MarkupNodeId,
    options: LoweringOptions,
) -> Result<WidgetPlan, LoweringError> {
    let mut lowering = Lowering::new(module, options);
    let root = lowering.lower_root(root)?;
    let plan = WidgetPlan::new(root, lowering.nodes);
    plan.validate().map_err(LoweringError::InvalidPlan)?;
    Ok(plan)
}

struct Lowering<'module> {
    module: &'module Module,
    options: LoweringOptions,
    nodes: Vec<PlanNode>,
    markup_nodes: HashMap<MarkupNodeId, PlanNodeId>,
    control_nodes: HashMap<ControlNodeId, PlanNodeId>,
}

impl<'module> Lowering<'module> {
    fn new(module: &'module Module, options: LoweringOptions) -> Self {
        Self {
            module,
            options,
            nodes: Vec::new(),
            markup_nodes: HashMap::new(),
            control_nodes: HashMap::new(),
        }
    }

    fn lower_root(&mut self, root: MarkupNodeId) -> Result<PlanNodeId, LoweringError> {
        let mut worklist = vec![PendingNode::Markup {
            id: root,
            state: VisitState::Enter,
        }];

        while let Some(node) = worklist.pop() {
            match node {
                PendingNode::Markup {
                    id,
                    state: VisitState::Enter,
                } => {
                    if self.markup_nodes.contains_key(&id) {
                        continue;
                    }
                    let node = self
                        .module
                        .markup_nodes()
                        .get(id)
                        .ok_or(LoweringError::MissingMarkupNode(id))?;
                    worklist.push(PendingNode::Markup {
                        id,
                        state: VisitState::Exit,
                    });
                    match &node.kind {
                        MarkupNodeKind::Element(element) => {
                            for child in element.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        MarkupNodeKind::Control(control) => {
                            worklist.push(PendingNode::Control {
                                id: *control,
                                state: VisitState::Enter,
                            });
                        }
                    }
                }
                PendingNode::Markup {
                    id,
                    state: VisitState::Exit,
                } => {
                    if self.markup_nodes.contains_key(&id) {
                        continue;
                    }
                    let node = self
                        .module
                        .markup_nodes()
                        .get(id)
                        .ok_or(LoweringError::MissingMarkupNode(id))?;
                    let plan_id = match &node.kind {
                        MarkupNodeKind::Element(element) => {
                            self.lower_element(id, node.span, element)?
                        }
                        MarkupNodeKind::Control(control) => {
                            self.control_nodes.get(control).copied().ok_or(
                                LoweringError::MissingLoweredControlBranch {
                                    parent: StableNodeId::Markup(id),
                                    branch: *control,
                                },
                            )?
                        }
                    };
                    self.markup_nodes.insert(id, plan_id);
                }
                PendingNode::Control {
                    id,
                    state: VisitState::Enter,
                } => {
                    if self.control_nodes.contains_key(&id) {
                        continue;
                    }
                    let node = self
                        .module
                        .control_nodes()
                        .get(id)
                        .ok_or(LoweringError::MissingControlNode(id))?;
                    worklist.push(PendingNode::Control {
                        id,
                        state: VisitState::Exit,
                    });
                    match node {
                        ControlNode::Show(show) => {
                            for child in show.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::Each(each) => {
                            if let Some(empty) = each.empty {
                                worklist.push(PendingNode::Control {
                                    id: empty,
                                    state: VisitState::Enter,
                                });
                            }
                            for child in each.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::Empty(empty) => {
                            for child in empty.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::Match(match_node) => {
                            for case in match_node.cases.iter().rev() {
                                worklist.push(PendingNode::Control {
                                    id: *case,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::Case(case) => {
                            for child in case.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::Fragment(fragment) => {
                            for child in fragment.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                        ControlNode::With(with_node) => {
                            for child in with_node.children.iter().rev() {
                                worklist.push(PendingNode::Markup {
                                    id: *child,
                                    state: VisitState::Enter,
                                });
                            }
                        }
                    }
                }
                PendingNode::Control {
                    id,
                    state: VisitState::Exit,
                } => {
                    if self.control_nodes.contains_key(&id) {
                        continue;
                    }
                    let node = self
                        .module
                        .control_nodes()
                        .get(id)
                        .ok_or(LoweringError::MissingControlNode(id))?;
                    let plan_id = self.lower_control(id, node)?;
                    self.control_nodes.insert(id, plan_id);
                }
            }
        }

        let root = self
            .markup_nodes
            .get(&root)
            .copied()
            .ok_or(LoweringError::MissingMarkupNode(root))?;
        if let Some(PlanNode {
            span,
            kind: PlanNodeKind::Group(group),
            ..
        }) = self.nodes.get(root.index())
        {
            return Err(LoweringError::UnexpectedRootChildGroup {
                group: widget_child_group_label(&group.widget, &group.group),
                span: *span,
            });
        }
        Ok(root)
    }

    fn lower_element(
        &mut self,
        id: MarkupNodeId,
        span: aivi_base::SourceSpan,
        element: &aivi_hir::MarkupElement,
    ) -> Result<PlanNodeId, LoweringError> {
        let stable_id = StableNodeId::Markup(id);
        if let Some((widget, group)) = split_widget_child_group(&element.name) {
            if !element.attributes.is_empty() {
                return Err(LoweringError::ChildGroupHasAttributes {
                    group: element.name.to_string(),
                    span,
                });
            }
            let children = self.child_ops_from_markup(stable_id, None, &element.children)?;
            return Ok(self.push_node(PlanNode {
                stable_id,
                span,
                kind: PlanNodeKind::Group(GroupNode {
                    widget,
                    group,
                    children,
                }),
            }));
        }
        let widget_name_str = crate::schema::widget_leaf_name(&element.name).to_string();
        if lookup_widget_schema(&element.name).is_none() {
            return Err(LoweringError::UnknownWidget {
                name: widget_name_str,
                span,
            });
        }
        let children =
            self.child_ops_from_markup(stable_id, Some(&element.name), &element.children)?;
        let (properties, event_hooks) =
            self.lower_attributes(stable_id, &element.name, &element.attributes)?;
        Ok(self.push_node(PlanNode {
            stable_id,
            span,
            kind: PlanNodeKind::Widget(WidgetNode {
                widget: element.name.clone(),
                properties,
                event_hooks,
                children,
            }),
        }))
    }

    fn lower_control(
        &mut self,
        id: ControlNodeId,
        node: &ControlNode,
    ) -> Result<PlanNodeId, LoweringError> {
        let stable_id = StableNodeId::Control(id);
        let kind = match node {
            ControlNode::Show(show) => {
                self.require_expr(show.when)?;
                if let Some(keep_mounted) = show.keep_mounted {
                    self.require_expr(keep_mounted)?;
                }
                PlanNodeKind::Show(ShowNode {
                    when: show.when,
                    mount: show
                        .keep_mounted
                        .map_or(ShowMountPolicy::UnmountWhenHidden, |decision| {
                            ShowMountPolicy::KeepMounted { decision }
                        }),
                    children: self.child_ops_from_markup(stable_id, None, &show.children)?,
                })
            }
            ControlNode::Each(each) => {
                self.require_expr(each.collection)?;
                self.require_binding(each.binding)?;
                let child_policy = match each.key {
                    Some(key) => {
                        self.require_expr(key)?;
                        RepeatedChildPolicy::Keyed {
                            key,
                            updates: ChildUpdateMode::Localized,
                        }
                    }
                    None => RepeatedChildPolicy::Positional {
                        updates: ChildUpdateMode::Localized,
                    },
                };
                let empty_branch = each
                    .empty
                    .map(|empty| self.control_plan_id(stable_id, empty))
                    .transpose()?;
                PlanNodeKind::Each(EachNode {
                    collection: each.collection,
                    binding: each.binding,
                    child_policy,
                    item_children: self.child_ops_from_markup(stable_id, None, &each.children)?,
                    empty_branch,
                })
            }
            ControlNode::Empty(empty) => PlanNodeKind::Empty(EmptyNode {
                children: self.child_ops_from_markup(stable_id, None, &empty.children)?,
            }),
            ControlNode::Match(match_node) => {
                self.require_expr(match_node.scrutinee)?;
                let mut cases = Vec::with_capacity(match_node.cases.len());
                for case in match_node.cases.iter() {
                    cases.push(self.control_plan_id(stable_id, *case)?);
                }
                let cases = NonEmpty::from_vec(cases)
                    .expect("validated HIR match controls always carry at least one case");
                PlanNodeKind::Match(MatchNode {
                    scrutinee: match_node.scrutinee,
                    cases,
                })
            }
            ControlNode::Case(case) => {
                self.require_pattern(case.pattern)?;
                PlanNodeKind::Case(CaseNode {
                    pattern: case.pattern,
                    children: self.child_ops_from_markup(stable_id, None, &case.children)?,
                })
            }
            ControlNode::Fragment(fragment) => PlanNodeKind::Fragment(FragmentNode {
                children: self.child_ops_from_markup(stable_id, None, &fragment.children)?,
            }),
            ControlNode::With(with_node) => {
                self.require_expr(with_node.value)?;
                self.require_binding(with_node.binding)?;
                PlanNodeKind::With(WithNode {
                    value: with_node.value,
                    binding: with_node.binding,
                    children: self.child_ops_from_markup(stable_id, None, &with_node.children)?,
                })
            }
        };

        Ok(self.push_node(PlanNode {
            stable_id,
            span: node.span(),
            kind,
        }))
    }

    fn lower_attributes(
        &self,
        owner: StableNodeId,
        widget: &aivi_hir::NamePath,
        attributes: &[MarkupAttribute],
    ) -> Result<(Vec<PropertyPlan>, Vec<EventHookPlan>), LoweringError> {
        let mut properties = Vec::new();
        let mut event_hooks = Vec::new();
        for (index, attribute) in attributes.iter().enumerate() {
            let site = AttributeSite {
                owner,
                index,
                span: attribute.span,
            };
            match &attribute.value {
                MarkupAttributeValue::ImplicitTrue => {
                    properties.push(PropertyPlan::Static(StaticPropertyPlan {
                        site,
                        name: attribute.name.clone(),
                        value: StaticPropertyValue::ImplicitTrue,
                    }));
                }
                MarkupAttributeValue::Text(text) => {
                    self.require_text(text)?;
                    if text.has_interpolation() {
                        properties.push(PropertyPlan::Setter(SetterBindingPlan {
                            site,
                            name: attribute.name.clone(),
                            source: SetterSource::InterpolatedText(text.clone()),
                            update: SetterUpdateStrategy::DirectSetter,
                            teardown: SetterTeardown::CancelSubscription,
                        }));
                    } else {
                        properties.push(PropertyPlan::Static(StaticPropertyPlan {
                            site,
                            name: attribute.name.clone(),
                            value: StaticPropertyValue::Text(text.clone()),
                        }));
                    }
                }
                MarkupAttributeValue::Expr(expr) => {
                    self.require_expr(*expr)?;
                    if self.options.lowers_as_event(widget, attribute) {
                        event_hooks.push(EventHookPlan {
                            site,
                            name: attribute.name.clone(),
                            handler: *expr,
                            hookup: EventHookStrategy::DirectSignal,
                            teardown: EventHookTeardown::DisconnectHandler,
                        });
                    } else {
                        properties.push(PropertyPlan::Setter(SetterBindingPlan {
                            site,
                            name: attribute.name.clone(),
                            source: SetterSource::Expr(*expr),
                            update: SetterUpdateStrategy::DirectSetter,
                            teardown: SetterTeardown::CancelSubscription,
                        }));
                    }
                }
            }
        }
        Ok((properties, event_hooks))
    }

    fn child_ops_from_markup(
        &self,
        parent: StableNodeId,
        widget: Option<&NamePath>,
        children: &[MarkupNodeId],
    ) -> Result<Vec<ChildOp>, LoweringError> {
        children
            .iter()
            .map(|child| {
                let plan_id = self.markup_nodes.get(child).copied().ok_or(
                    LoweringError::MissingLoweredMarkupChild {
                        parent,
                        child: *child,
                    },
                )?;
                if let Some(PlanNode {
                    span,
                    kind: PlanNodeKind::Group(group_node),
                    ..
                }) = self.nodes.get(plan_id.index())
                {
                    let group_label =
                        widget_child_group_label(&group_node.widget, &group_node.group);
                    let Some(parent_widget) = widget else {
                        return Err(LoweringError::MisplacedChildGroup {
                            parent,
                            group: group_label,
                            span: *span,
                        });
                    };
                    if !same_name_path(&group_node.widget, parent_widget) {
                        return Err(LoweringError::MismatchedChildGroupOwner {
                            parent_widget: parent_widget.to_string(),
                            group: group_label,
                            span: *span,
                        });
                    }
                    let Some(schema) = lookup_widget_schema(parent_widget) else {
                        return Err(LoweringError::UnknownWidget {
                            name: crate::schema::widget_leaf_name(parent_widget).to_string(),
                            span: *span,
                        });
                    };
                    if schema.child_group(group_node.group.text()).is_none() {
                        return Err(LoweringError::UnknownWidgetChildGroup {
                            widget: parent_widget.to_string(),
                            group: group_label,
                            span: *span,
                        });
                    }
                }
                Ok(ChildOp::Append(plan_id))
            })
            .collect()
    }

    fn control_plan_id(
        &self,
        parent: StableNodeId,
        branch: ControlNodeId,
    ) -> Result<PlanNodeId, LoweringError> {
        self.control_nodes
            .get(&branch)
            .copied()
            .ok_or(LoweringError::MissingLoweredControlBranch { parent, branch })
    }

    fn require_expr(&self, expr: ExprId) -> Result<(), LoweringError> {
        self.module
            .exprs()
            .get(expr)
            .map(|_| ())
            .ok_or(LoweringError::MissingExpr(expr))
    }

    fn require_binding(&self, binding: BindingId) -> Result<(), LoweringError> {
        self.module
            .bindings()
            .get(binding)
            .map(|_| ())
            .ok_or(LoweringError::MissingBinding(binding))
    }

    fn require_pattern(&self, pattern: PatternId) -> Result<(), LoweringError> {
        self.module
            .patterns()
            .get(pattern)
            .map(|_| ())
            .ok_or(LoweringError::MissingPattern(pattern))
    }

    fn require_text(&self, text: &TextLiteral) -> Result<(), LoweringError> {
        for segment in &text.segments {
            if let TextSegment::Interpolation(interpolation) = segment {
                self.require_expr(interpolation.expr)?;
            }
        }
        Ok(())
    }

    fn push_node(&mut self, node: PlanNode) -> PlanNodeId {
        let id = PlanNodeId::new(self.nodes.len() as u32);
        self.nodes.push(node);
        id
    }
}

fn split_widget_child_group(name: &NamePath) -> Option<(NamePath, Name)> {
    let mut segments = name.segments().iter().cloned().collect::<Vec<_>>();
    if segments.len() < 2 {
        return None;
    }
    let group = segments
        .pop()
        .expect("paths with at least two segments always have a final child-group segment");
    let widget =
        NamePath::from_vec(segments).expect("paths with at least one widget segment remain valid");
    Some((widget, group))
}

fn widget_child_group_label(widget: &NamePath, group: &Name) -> String {
    format!("{}.{}", widget, group.text())
}

fn same_name_path(left: &NamePath, right: &NamePath) -> bool {
    left.segments()
        .iter()
        .map(Name::text)
        .eq(right.segments().iter().map(Name::text))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisitState {
    Enter,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingNode {
    Markup {
        id: MarkupNodeId,
        state: VisitState,
    },
    Control {
        id: ControlNodeId,
        state: VisitState,
    },
}
