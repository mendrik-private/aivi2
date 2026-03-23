use std::{collections::BTreeMap, error::Error, fmt};

use aivi_runtime::InputHandle;

use crate::{
    GtkBridgeExecutionError, GtkBridgeGraph, GtkBridgeNodeKind, GtkBridgeNodeRef, GtkChildGroup,
    GtkChildGroupEdit, GtkCollectionKey, GtkEachEdit, GtkRepeatedChildEdit,
    GtkRepeatedChildIdentity, GtkShowState, PlanNodeTag, RuntimeEventBinding,
    RuntimePropertyBinding, RuntimeSetterBinding, StaticPropertyPlan,
};

/// Execute one lowered GTK bridge graph through an explicit host boundary.
///
/// This stays on the narrowest coherent seam before the concrete gtk4/libadwaita host lands:
///
/// - widget instances are allocated once per mounted execution path,
/// - direct setters replay cached values onto newly mounted widgets,
/// - event hookups expose explicit routes instead of hiding callback wiring, and
/// - control-node edits execute as concrete localized child operations rather than VDOM diffing.
pub trait GtkRuntimeHost<V> {
    type Widget: Clone + fmt::Debug + PartialEq + Eq;
    type EventHandle: Clone + fmt::Debug + PartialEq + Eq;
    type Error;

    fn create_widget(
        &mut self,
        instance: &GtkNodeInstance,
        widget: &aivi_hir::NamePath,
    ) -> Result<Self::Widget, Self::Error>;

    fn apply_static_property(
        &mut self,
        widget: &Self::Widget,
        property: &StaticPropertyPlan,
    ) -> Result<(), Self::Error>;

    fn apply_dynamic_property(
        &mut self,
        widget: &Self::Widget,
        binding: &RuntimeSetterBinding,
        value: &V,
    ) -> Result<(), Self::Error>;

    fn connect_event(
        &mut self,
        widget: &Self::Widget,
        route: &GtkEventRoute,
    ) -> Result<Self::EventHandle, Self::Error>;

    fn disconnect_event(
        &mut self,
        widget: &Self::Widget,
        event: &Self::EventHandle,
    ) -> Result<(), Self::Error>;

    fn insert_children(
        &mut self,
        parent: &Self::Widget,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error>;

    fn remove_children(
        &mut self,
        parent: &Self::Widget,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error>;

    fn move_children(
        &mut self,
        parent: &Self::Widget,
        from: usize,
        count: usize,
        to: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error>;

    fn set_widget_visibility(
        &mut self,
        widget: &Self::Widget,
        visible: bool,
    ) -> Result<(), Self::Error>;

    fn release_widget(&mut self, widget: Self::Widget) -> Result<(), Self::Error>;
}

pub trait GtkEventSink<V> {
    type Error;

    fn dispatch_event(&mut self, route: &GtkEventRoute, value: V) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkExecutionPath {
    segments: Box<[GtkExecutionPathSegment]>,
}

impl GtkExecutionPath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn segments(&self) -> &[GtkExecutionPathSegment] {
        &self.segments
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn pushed(&self, each: GtkBridgeNodeRef, identity: GtkRepeatedChildIdentity) -> Self {
        let mut segments = self.segments.to_vec();
        segments.push(GtkExecutionPathSegment { each, identity });
        Self {
            segments: segments.into_boxed_slice(),
        }
    }
}

impl fmt::Display for GtkExecutionPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            return f.write_str("root");
        }
        for (index, segment) in self.segments.iter().enumerate() {
            if index > 0 {
                f.write_str("/")?;
            }
            write!(f, "{}#{}", segment.each, segment.identity)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkExecutionPathSegment {
    pub each: GtkBridgeNodeRef,
    pub identity: GtkRepeatedChildIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkNodeInstance {
    pub node: GtkBridgeNodeRef,
    pub path: GtkExecutionPath,
}

impl GtkNodeInstance {
    pub fn root(node: GtkBridgeNodeRef) -> Self {
        Self {
            node,
            path: GtkExecutionPath::root(),
        }
    }

    pub fn with_path(node: GtkBridgeNodeRef, path: GtkExecutionPath) -> Self {
        Self { node, path }
    }

    pub fn pushed(&self, each: GtkBridgeNodeRef, identity: GtkRepeatedChildIdentity) -> Self {
        Self {
            node: self.node,
            path: self.path.pushed(each, identity),
        }
    }
}

impl fmt::Display for GtkNodeInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.node, self.path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkEventRouteId(u32);

impl GtkEventRouteId {
    pub const fn as_raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for GtkEventRouteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gtk-event-route:{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkEventRoute {
    pub id: GtkEventRouteId,
    pub instance: GtkNodeInstance,
    pub binding: RuntimeEventBinding,
}

#[derive(Debug)]
pub enum GtkExecutorError<E> {
    Bridge(GtkBridgeExecutionError),
    Host(E),
    MissingInstance {
        instance: GtkNodeInstance,
    },
    DuplicateInstance {
        instance: GtkNodeInstance,
    },
    UnexpectedNodeKind {
        instance: GtkNodeInstance,
        expected: PlanNodeTag,
        found: PlanNodeTag,
    },
    ChildIndexOutOfRange {
        parent: GtkNodeInstance,
        index: usize,
        child_count: usize,
    },
    ChildAlreadyAttached {
        parent: GtkNodeInstance,
        child: GtkNodeInstance,
    },
    ChildMissing {
        parent: GtkNodeInstance,
        child: GtkNodeInstance,
    },
    UnknownSetterInput {
        input: InputHandle,
    },
    RepeatedItemIndexOutOfRange {
        each: GtkNodeInstance,
        index: usize,
        item_count: usize,
    },
    RepeatedItemIdentityMismatch {
        each: GtkNodeInstance,
        index: usize,
        expected: GtkRepeatedChildIdentity,
        found: GtkRepeatedChildIdentity,
    },
    MissingEventRoute {
        route: GtkEventRouteId,
    },
}

impl<E: fmt::Display> fmt::Display for GtkExecutorError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bridge(error) => write!(f, "GTK executor bridge failure: {error}"),
            Self::Host(error) => write!(f, "GTK host failure: {error}"),
            Self::MissingInstance { instance } => {
                write!(f, "GTK executor instance {instance} is not mounted")
            }
            Self::DuplicateInstance { instance } => {
                write!(f, "GTK executor instance {instance} is already mounted")
            }
            Self::UnexpectedNodeKind {
                instance,
                expected,
                found,
            } => write!(
                f,
                "GTK executor instance {instance} expected {expected:?}, found {found:?}"
            ),
            Self::ChildIndexOutOfRange {
                parent,
                index,
                child_count,
            } => write!(
                f,
                "GTK executor parent {parent} requested child index {index}, but only {child_count} child instance(s) exist"
            ),
            Self::ChildAlreadyAttached { parent, child } => write!(
                f,
                "GTK executor child {child} is already attached under parent {parent}"
            ),
            Self::ChildMissing { parent, child } => write!(
                f,
                "GTK executor parent {parent} does not contain child {child}"
            ),
            Self::UnknownSetterInput { input } => write!(
                f,
                "GTK executor input {} does not belong to a dynamic setter binding",
                input.as_raw()
            ),
            Self::RepeatedItemIndexOutOfRange {
                each,
                index,
                item_count,
            } => write!(
                f,
                "GTK executor each instance {each} requested item index {index}, but only {item_count} item(s) exist"
            ),
            Self::RepeatedItemIdentityMismatch {
                each,
                index,
                expected,
                found,
            } => write!(
                f,
                "GTK executor each instance {each} expected item {index} to carry identity {expected}, found {found}"
            ),
            Self::MissingEventRoute { route } => {
                write!(f, "GTK executor event route {route} is not connected")
            }
        }
    }
}

impl<E> Error for GtkExecutorError<E> where E: Error + 'static {}

#[derive(Debug)]
pub enum GtkEventDispatchError<E> {
    MissingRoute { route: GtkEventRouteId },
    Sink(E),
}

impl<E: fmt::Display> fmt::Display for GtkEventDispatchError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRoute { route } => {
                write!(f, "GTK executor event route {route} is not connected")
            }
            Self::Sink(error) => write!(f, "GTK event sink failure: {error}"),
        }
    }
}

impl<E> Error for GtkEventDispatchError<E> where E: Error + 'static {}

pub struct GtkRuntimeExecutor<H, V>
where
    H: GtkRuntimeHost<V>,
    V: Clone,
{
    bridge: GtkBridgeGraph,
    host: H,
    root: GtkNodeInstance,
    setter_sites: BTreeMap<InputHandle, GtkSetterSite>,
    values: BTreeMap<InputHandle, V>,
    instances: BTreeMap<GtkNodeInstance, MountedNode<H::Widget>>,
    routes: BTreeMap<GtkEventRouteId, MountedRoute<H::Widget, H::EventHandle>>,
    next_route: u32,
}

impl<H, V> GtkRuntimeExecutor<H, V>
where
    H: GtkRuntimeHost<V>,
    V: Clone,
{
    pub fn new(bridge: GtkBridgeGraph, host: H) -> Result<Self, GtkExecutorError<H::Error>> {
        Self::new_with_values(bridge, host, std::iter::empty())
    }

    pub fn new_with_values<I>(
        bridge: GtkBridgeGraph,
        host: H,
        initial_values: I,
    ) -> Result<Self, GtkExecutorError<H::Error>>
    where
        I: IntoIterator<Item = (InputHandle, V)>,
    {
        let setter_sites = collect_setter_sites(&bridge);
        let root = GtkNodeInstance::root(bridge.root());
        let mut executor = Self {
            bridge,
            host,
            root: root.clone(),
            setter_sites,
            values: initial_values.into_iter().collect(),
            instances: BTreeMap::new(),
            routes: BTreeMap::new(),
            next_route: 0,
        };
        executor.mount_subtree(root.node, None, GtkExecutionPath::root())?;
        Ok(executor)
    }

    pub fn bridge(&self) -> &GtkBridgeGraph {
        &self.bridge
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    pub fn into_host(self) -> H {
        self.host
    }

    pub fn root_instance(&self) -> GtkNodeInstance {
        self.root.clone()
    }

    pub fn root_widgets(&self) -> Result<Vec<H::Widget>, GtkExecutorError<H::Error>> {
        self.instance_root_widgets(&self.root)
    }

    pub fn is_mounted(&self, instance: &GtkNodeInstance) -> bool {
        self.instances.contains_key(instance)
    }

    pub fn widget_handle(
        &self,
        instance: &GtkNodeInstance,
    ) -> Result<&H::Widget, GtkExecutorError<H::Error>> {
        match &self.instance_state(instance)?.kind {
            MountedNodeKind::Widget(widget) => Ok(&widget.handle),
            _ => Err(GtkExecutorError::UnexpectedNodeKind {
                instance: instance.clone(),
                expected: PlanNodeTag::Widget,
                found: self.bridge_tag(instance.node)?,
            }),
        }
    }

    pub fn event_routes(&self) -> Vec<GtkEventRoute> {
        self.routes
            .values()
            .map(|route| route.route.clone())
            .collect::<Vec<_>>()
    }

    pub fn event_routes_for_instance(&self, instance: &GtkNodeInstance) -> Vec<GtkEventRoute> {
        self.routes
            .values()
            .filter(|route| route.route.instance == *instance)
            .map(|route| route.route.clone())
            .collect::<Vec<_>>()
    }

    pub fn set_property(
        &mut self,
        input: InputHandle,
        value: V,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let site = self
            .setter_sites
            .get(&input)
            .cloned()
            .ok_or(GtkExecutorError::UnknownSetterInput { input })?;
        self.values.insert(input, value.clone());
        let targets = self
            .instances
            .keys()
            .filter(|instance| instance.node == site.node)
            .cloned()
            .collect::<Vec<_>>();
        for instance in targets {
            let handle = self.widget_handle(&instance)?.clone();
            self.host
                .apply_dynamic_property(&handle, &site.binding, &value)
                .map_err(GtkExecutorError::Host)?;
        }
        Ok(())
    }

    pub fn dispatch_event<S>(
        &self,
        route: GtkEventRouteId,
        value: V,
        sink: &mut S,
    ) -> Result<(), GtkEventDispatchError<S::Error>>
    where
        S: GtkEventSink<V>,
    {
        let route = self
            .routes
            .get(&route)
            .ok_or(GtkEventDispatchError::MissingRoute { route })?;
        sink.dispatch_event(&route.route, value)
            .map_err(GtkEventDispatchError::Sink)
    }

    pub fn update_show(
        &mut self,
        instance: &GtkNodeInstance,
        when: bool,
        keep_mounted: bool,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let previous = match &self.instance_state(instance)?.kind {
            MountedNodeKind::Show(show) => show.state,
            _ => {
                return Err(GtkExecutorError::UnexpectedNodeKind {
                    instance: instance.clone(),
                    expected: PlanNodeTag::Show,
                    found: self.bridge_tag(instance.node)?,
                });
            }
        };
        let transition = self
            .bridge
            .show_transition(instance.node, previous, when, keep_mounted)
            .map_err(GtkExecutorError::Bridge)?;
        for edit in transition.edits.iter() {
            self.apply_group_edit(instance, edit)?;
        }
        match &mut self.instance_state_mut(instance)?.kind {
            MountedNodeKind::Show(show) => show.state = transition.next,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn update_match(
        &mut self,
        instance: &GtkNodeInstance,
        next_case: usize,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let previous_case = match &self.instance_state(instance)?.kind {
            MountedNodeKind::Match(match_state) => match_state.active_case,
            _ => {
                return Err(GtkExecutorError::UnexpectedNodeKind {
                    instance: instance.clone(),
                    expected: PlanNodeTag::Match,
                    found: self.bridge_tag(instance.node)?,
                });
            }
        };
        let transition = self
            .bridge
            .match_transition(instance.node, previous_case, next_case)
            .map_err(GtkExecutorError::Bridge)?;
        for edit in transition.edits.iter() {
            self.apply_group_edit(instance, edit)?;
        }
        match &mut self.instance_state_mut(instance)?.kind {
            MountedNodeKind::Match(match_state) => {
                match_state.active_case = Some(next_case);
                match_state.active_case_instance = Some(GtkNodeInstance::with_path(
                    transition.next.case,
                    instance.path.clone(),
                ));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn update_each_positional(
        &mut self,
        instance: &GtkNodeInstance,
        next_len: usize,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let (previous_len, initialized) = match &self.instance_state(instance)?.kind {
            MountedNodeKind::Each(each) => (each.items.len(), each.initialized),
            _ => {
                return Err(GtkExecutorError::UnexpectedNodeKind {
                    instance: instance.clone(),
                    expected: PlanNodeTag::Each,
                    found: self.bridge_tag(instance.node)?,
                });
            }
        };
        let transition = self
            .bridge
            .each_positional_transition(instance.node, previous_len, next_len)
            .map_err(GtkExecutorError::Bridge)?;
        self.apply_each_transition(instance, &transition)?;
        if !initialized && next_len == 0 {
            self.mount_initial_empty_branch(instance)?;
        }
        match &mut self.instance_state_mut(instance)?.kind {
            MountedNodeKind::Each(each) => each.initialized = true,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn update_each_keyed(
        &mut self,
        instance: &GtkNodeInstance,
        next: &[GtkCollectionKey],
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let (previous, initialized) = match &self.instance_state(instance)?.kind {
            MountedNodeKind::Each(each) => (
                each.items
                    .iter()
                    .map(|item| match &item.identity {
                        GtkRepeatedChildIdentity::Keyed(key) => key.clone(),
                        GtkRepeatedChildIdentity::Positional(index) => {
                            GtkCollectionKey::new(index.to_string())
                        }
                    })
                    .collect::<Vec<_>>(),
                each.initialized,
            ),
            _ => {
                return Err(GtkExecutorError::UnexpectedNodeKind {
                    instance: instance.clone(),
                    expected: PlanNodeTag::Each,
                    found: self.bridge_tag(instance.node)?,
                });
            }
        };
        let transition = self
            .bridge
            .each_keyed_transition(instance.node, &previous, next)
            .map_err(GtkExecutorError::Bridge)?;
        self.apply_each_transition(instance, &transition)?;
        if !initialized && next.is_empty() {
            self.mount_initial_empty_branch(instance)?;
        }
        match &mut self.instance_state_mut(instance)?.kind {
            MountedNodeKind::Each(each) => each.initialized = true,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn apply_each_transition(
        &mut self,
        instance: &GtkNodeInstance,
        transition: &crate::GtkEachTransition,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        for edit in transition.edits.iter() {
            match edit {
                GtkEachEdit::Group(group) => self.apply_group_edit(instance, group)?,
                GtkEachEdit::Item(item) => self.apply_each_item_edit(instance, item)?,
            }
        }
        Ok(())
    }

    fn mount_initial_empty_branch(
        &mut self,
        instance: &GtkNodeInstance,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let empty_ref = match &self.bridge_node(instance.node)?.kind {
            GtkBridgeNodeKind::Each(each) => each.empty_branch.as_ref().map(|branch| branch.empty),
            _ => None,
        };
        let Some(empty_ref) = empty_ref else {
            return Ok(());
        };
        let already_mounted = match &self.instance_state(instance)?.kind {
            MountedNodeKind::Each(each) => each.empty_branch.is_some(),
            _ => false,
        };
        if already_mounted {
            return Ok(());
        }
        let empty_instance =
            self.mount_subtree(empty_ref, Some(instance.clone()), instance.path.clone())?;
        let child_index = self.instance_state(instance)?.children.len();
        self.attach_existing_child(instance, child_index, empty_instance.clone())?;
        match &mut self.instance_state_mut(instance)?.kind {
            MountedNodeKind::Each(each) => each.empty_branch = Some(empty_instance),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn apply_group_edit(
        &mut self,
        context: &GtkNodeInstance,
        edit: &GtkChildGroupEdit,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        match edit {
            GtkChildGroupEdit::Mount { group } => self.mount_group(context, group),
            GtkChildGroupEdit::Unmount { group } => self.unmount_group(context, group),
            GtkChildGroupEdit::SetVisibility { visible, .. } => {
                let widgets = self.instance_root_widgets(context)?;
                for widget in widgets {
                    self.host
                        .set_widget_visibility(&widget, *visible)
                        .map_err(GtkExecutorError::Host)?;
                }
                Ok(())
            }
        }
    }

    fn mount_group(
        &mut self,
        context: &GtkNodeInstance,
        group: &GtkChildGroup,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        if group.owner == context.node {
            let mut insert_index = self.instance_state(context)?.children.len();
            for &root in group.roots.iter() {
                let child =
                    self.mount_subtree(root, Some(context.clone()), context.path.clone())?;
                self.attach_existing_child(context, insert_index, child)?;
                insert_index += 1;
            }
            return Ok(());
        }

        let child = GtkNodeInstance::with_path(group.owner, context.path.clone());
        if self.instances.contains_key(&child) {
            return Ok(());
        }
        let mounted =
            self.mount_subtree(group.owner, Some(context.clone()), context.path.clone())?;
        let index = self.instance_state(context)?.children.len();
        self.attach_existing_child(context, index, mounted)?;
        match &mut self.instance_state_mut(context)?.kind {
            MountedNodeKind::Match(match_state)
                if group.kind == crate::GtkChildGroupKind::CaseBody =>
            {
                match_state.active_case_instance = Some(child);
            }
            MountedNodeKind::Each(each)
                if group.kind == crate::GtkChildGroupKind::EachEmptyBranch =>
            {
                each.empty_branch = Some(child);
            }
            _ => {}
        }
        Ok(())
    }

    fn unmount_group(
        &mut self,
        context: &GtkNodeInstance,
        group: &GtkChildGroup,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        if group.owner == context.node {
            let child_count = self.instance_state(context)?.children.len();
            if child_count == 0 {
                return Ok(());
            }
            self.detach_existing_child_block(context, 0, child_count)?;
            return Ok(());
        }

        let child = GtkNodeInstance::with_path(group.owner, context.path.clone());
        let Some(index) = self.find_child_index_opt(context, &child)? else {
            return Ok(());
        };
        self.detach_existing_child_block(context, index, 1)?;
        match &mut self.instance_state_mut(context)?.kind {
            MountedNodeKind::Match(match_state)
                if group.kind == crate::GtkChildGroupKind::CaseBody =>
            {
                match_state.active_case_instance = None;
            }
            MountedNodeKind::Each(each)
                if group.kind == crate::GtkChildGroupKind::EachEmptyBranch =>
            {
                each.empty_branch = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_each_item_edit(
        &mut self,
        each: &GtkNodeInstance,
        edit: &GtkRepeatedChildEdit,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        self.expect_node_kind(each, PlanNodeTag::Each)?;
        match edit {
            GtkRepeatedChildEdit::Insert { item, to } => {
                let child_index = self.each_child_insert_index(each, *to)?;
                let item_path = each.path.pushed(each.node, item.identity.clone());
                let mut roots = Vec::with_capacity(item.template.roots.len());
                for &root in item.template.roots.iter() {
                    roots.push(self.mount_subtree(root, Some(each.clone()), item_path.clone())?);
                }
                for (offset, root) in roots.iter().cloned().enumerate() {
                    self.attach_existing_child(each, child_index + offset, root)?;
                }
                match &mut self.instance_state_mut(each)?.kind {
                    MountedNodeKind::Each(each_state) => each_state.items.insert(
                        *to,
                        MountedEachItem {
                            identity: item.identity.clone(),
                            roots,
                        },
                    ),
                    _ => unreachable!(),
                }
            }
            GtkRepeatedChildEdit::Move { item, from, to } => {
                let (actual_identity, root_count, child_index, target_child_index) = {
                    let each_state = match &self.instance_state(each)?.kind {
                        MountedNodeKind::Each(each_state) => each_state,
                        _ => unreachable!(),
                    };
                    let mounted = each_state.items.get(*from).ok_or(
                        GtkExecutorError::RepeatedItemIndexOutOfRange {
                            each: each.clone(),
                            index: *from,
                            item_count: each_state.items.len(),
                        },
                    )?;
                    let actual_identity = mounted.identity.clone();
                    if actual_identity != item.identity {
                        return Err(GtkExecutorError::RepeatedItemIdentityMismatch {
                            each: each.clone(),
                            index: *from,
                            expected: item.identity.clone(),
                            found: actual_identity,
                        });
                    }
                    let root_count = mounted.roots.len();
                    let child_index = self.each_child_insert_index(each, *from)?;
                    let target_child_index =
                        self.each_child_insert_index_after_removal(each, each_state, *from, *to)?;
                    (
                        mounted.identity.clone(),
                        root_count,
                        child_index,
                        target_child_index,
                    )
                };
                self.move_existing_child_block(each, child_index, root_count, target_child_index)?;
                match &mut self.instance_state_mut(each)?.kind {
                    MountedNodeKind::Each(each_state) => {
                        let moved = each_state.items.remove(*from);
                        debug_assert_eq!(moved.identity, actual_identity);
                        each_state.items.insert(*to, moved);
                    }
                    _ => unreachable!(),
                }
            }
            GtkRepeatedChildEdit::Remove { item, from } => {
                let (actual_identity, root_count, child_index) = {
                    let each_state = match &self.instance_state(each)?.kind {
                        MountedNodeKind::Each(each_state) => each_state,
                        _ => unreachable!(),
                    };
                    let mounted = each_state.items.get(*from).ok_or(
                        GtkExecutorError::RepeatedItemIndexOutOfRange {
                            each: each.clone(),
                            index: *from,
                            item_count: each_state.items.len(),
                        },
                    )?;
                    let actual_identity = mounted.identity.clone();
                    if actual_identity != item.identity {
                        return Err(GtkExecutorError::RepeatedItemIdentityMismatch {
                            each: each.clone(),
                            index: *from,
                            expected: item.identity.clone(),
                            found: actual_identity,
                        });
                    }
                    (
                        mounted.identity.clone(),
                        mounted.roots.len(),
                        self.each_child_insert_index(each, *from)?,
                    )
                };
                self.detach_existing_child_block(each, child_index, root_count)?;
                match &mut self.instance_state_mut(each)?.kind {
                    MountedNodeKind::Each(each_state) => {
                        let removed = each_state.items.remove(*from);
                        debug_assert_eq!(removed.identity, actual_identity);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }

    fn each_child_insert_index(
        &self,
        each: &GtkNodeInstance,
        item_index: usize,
    ) -> Result<usize, GtkExecutorError<H::Error>> {
        let each_state = match &self.instance_state(each)?.kind {
            MountedNodeKind::Each(each_state) => each_state,
            _ => unreachable!(),
        };
        if item_index > each_state.items.len() {
            return Err(GtkExecutorError::RepeatedItemIndexOutOfRange {
                each: each.clone(),
                index: item_index,
                item_count: each_state.items.len(),
            });
        }
        let mut child_index = usize::from(each_state.empty_branch.is_some());
        for item in each_state.items.iter().take(item_index) {
            child_index += item.roots.len();
        }
        Ok(child_index)
    }

    fn each_child_insert_index_after_removal(
        &self,
        each: &GtkNodeInstance,
        each_state: &MountedEach,
        from: usize,
        to: usize,
    ) -> Result<usize, GtkExecutorError<H::Error>> {
        if from >= each_state.items.len() {
            return Err(GtkExecutorError::RepeatedItemIndexOutOfRange {
                each: each.clone(),
                index: from,
                item_count: each_state.items.len(),
            });
        }
        if to > each_state.items.len().saturating_sub(1) {
            return Err(GtkExecutorError::RepeatedItemIndexOutOfRange {
                each: each.clone(),
                index: to,
                item_count: each_state.items.len().saturating_sub(1),
            });
        }
        let mut lengths = each_state
            .items
            .iter()
            .map(|item| item.roots.len())
            .collect::<Vec<_>>();
        lengths.remove(from);
        let mut child_index = usize::from(each_state.empty_branch.is_some());
        child_index += lengths.into_iter().take(to).sum::<usize>();
        Ok(child_index)
    }

    fn mount_subtree(
        &mut self,
        node: GtkBridgeNodeRef,
        _parent: Option<GtkNodeInstance>,
        path: GtkExecutionPath,
    ) -> Result<GtkNodeInstance, GtkExecutorError<H::Error>> {
        let root = GtkNodeInstance::with_path(node, path);
        let mut stack = vec![MountFrame::enter(root.clone())];
        while let Some(frame) = stack.pop() {
            match frame.phase {
                MountPhase::Enter => {
                    if self.instances.contains_key(&frame.instance) {
                        return Err(GtkExecutorError::DuplicateInstance {
                            instance: frame.instance,
                        });
                    }
                    let bridge_kind = self.bridge_node(frame.instance.node)?.kind.clone();
                    let fixed_children = fixed_children(&bridge_kind);
                    let state = self.instantiate_node(&frame.instance, &bridge_kind)?;
                    self.instances.insert(frame.instance.clone(), state);
                    stack.push(MountFrame::exit(
                        frame.instance.clone(),
                        fixed_children.clone(),
                    ));
                    for child in fixed_children.into_iter().rev() {
                        stack.push(MountFrame::enter(GtkNodeInstance::with_path(
                            child,
                            frame.instance.path.clone(),
                        )));
                    }
                }
                MountPhase::Exit { fixed_children } => {
                    for child in fixed_children {
                        let child = GtkNodeInstance::with_path(child, frame.instance.path.clone());
                        let index = self.instance_state(&frame.instance)?.children.len();
                        self.attach_existing_child(&frame.instance, index, child)?;
                    }
                }
            }
        }
        Ok(root)
    }

    fn instantiate_node(
        &mut self,
        instance: &GtkNodeInstance,
        kind: &GtkBridgeNodeKind,
    ) -> Result<MountedNode<H::Widget>, GtkExecutorError<H::Error>> {
        match kind {
            GtkBridgeNodeKind::Widget(widget) => {
                let handle = self
                    .host
                    .create_widget(instance, &widget.widget)
                    .map_err(GtkExecutorError::Host)?;
                for property in widget.properties.iter() {
                    match property {
                        RuntimePropertyBinding::Static(property) => {
                            self.host
                                .apply_static_property(&handle, property)
                                .map_err(GtkExecutorError::Host)?;
                        }
                        RuntimePropertyBinding::Setter(binding) => {
                            if let Some(value) = self.values.get(&binding.input).cloned() {
                                self.host
                                    .apply_dynamic_property(&handle, binding, &value)
                                    .map_err(GtkExecutorError::Host)?;
                            }
                        }
                    }
                }
                let mut event_routes = Vec::with_capacity(widget.event_hooks.len());
                for binding in widget.event_hooks.iter().cloned() {
                    let route_id = self.next_event_route_id();
                    let route = GtkEventRoute {
                        id: route_id,
                        instance: instance.clone(),
                        binding,
                    };
                    let event = self
                        .host
                        .connect_event(&handle, &route)
                        .map_err(GtkExecutorError::Host)?;
                    self.routes.insert(
                        route_id,
                        MountedRoute {
                            route,
                            widget: handle.clone(),
                            handle: event,
                        },
                    );
                    event_routes.push(route_id);
                }
                Ok(MountedNode {
                    parent: None,
                    children: Vec::new(),
                    root_widgets: vec![handle.clone()],
                    kind: MountedNodeKind::Widget(MountedWidget {
                        handle,
                        event_routes,
                    }),
                })
            }
            GtkBridgeNodeKind::Show(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Show(MountedShow {
                    state: GtkShowState::Unmounted,
                }),
            }),
            GtkBridgeNodeKind::Each(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Each(MountedEach {
                    initialized: false,
                    empty_branch: None,
                    items: Vec::new(),
                }),
            }),
            GtkBridgeNodeKind::Match(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Match(MountedMatch {
                    active_case: None,
                    active_case_instance: None,
                }),
            }),
            GtkBridgeNodeKind::Empty(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Structural,
            }),
            GtkBridgeNodeKind::Case(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Structural,
            }),
            GtkBridgeNodeKind::Fragment(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Structural,
            }),
            GtkBridgeNodeKind::With(_) => Ok(MountedNode {
                parent: None,
                children: Vec::new(),
                root_widgets: Vec::new(),
                kind: MountedNodeKind::Structural,
            }),
        }
    }

    fn attach_existing_child(
        &mut self,
        parent: &GtkNodeInstance,
        index: usize,
        child: GtkNodeInstance,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let child_count = self.instance_state(parent)?.children.len();
        if index > child_count {
            return Err(GtkExecutorError::ChildIndexOutOfRange {
                parent: parent.clone(),
                index,
                child_count,
            });
        }
        if self.instance_state(parent)?.children.contains(&child) {
            return Err(GtkExecutorError::ChildAlreadyAttached {
                parent: parent.clone(),
                child,
            });
        }
        let local_offset =
            self.root_widget_offset(&self.instance_state(parent)?.children, index)?;
        let block_widgets = self.instance_root_widgets(&child)?;
        if let Some(container) = self.widget_container_ancestor(parent)? {
            if !block_widgets.is_empty() {
                let abs_index = self.absolute_offset_in_ancestor(parent, local_offset)?;
                let widget = self.widget_handle(&container)?.clone();
                self.host
                    .insert_children(&widget, abs_index, &block_widgets)
                    .map_err(GtkExecutorError::Host)?;
            }
        }
        {
            let parent_state = self.instance_state_mut(parent)?;
            parent_state.children.insert(index, child.clone());
        }
        self.instance_state_mut(&child)?.parent = Some(parent.clone());
        self.refresh_root_widgets_from(parent.clone())
    }

    fn detach_existing_child_block(
        &mut self,
        parent: &GtkNodeInstance,
        start: usize,
        count: usize,
    ) -> Result<Vec<GtkNodeInstance>, GtkExecutorError<H::Error>> {
        let (child_count, removed, block_widgets, local_offset) = {
            let parent_state = self.instance_state(parent)?;
            let child_count = parent_state.children.len();
            if start > child_count || start + count > child_count {
                return Err(GtkExecutorError::ChildIndexOutOfRange {
                    parent: parent.clone(),
                    index: start,
                    child_count,
                });
            }
            let removed = parent_state.children[start..start + count].to_vec();
            let block_widgets = self.root_widgets_for_block(&removed)?;
            let local_offset = self.root_widget_offset(&parent_state.children, start)?;
            (child_count, removed, block_widgets, local_offset)
        };
        debug_assert!(start <= child_count);
        if let Some(container) = self.widget_container_ancestor(parent)? {
            if !block_widgets.is_empty() {
                let abs_index = self.absolute_offset_in_ancestor(parent, local_offset)?;
                let widget = self.widget_handle(&container)?.clone();
                self.host
                    .remove_children(&widget, abs_index, &block_widgets)
                    .map_err(GtkExecutorError::Host)?;
            }
        }
        self.instance_state_mut(parent)?
            .children
            .drain(start..start + count);
        self.refresh_root_widgets_from(parent.clone())?;
        self.teardown_subtrees(removed.clone())?;
        Ok(removed)
    }

    fn move_existing_child_block(
        &mut self,
        parent: &GtkNodeInstance,
        start: usize,
        count: usize,
        to: usize,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let (child_count, block_widgets, local_from, local_to) = {
            let parent_state = self.instance_state(parent)?;
            let child_count = parent_state.children.len();
            if start > child_count || start + count > child_count {
                return Err(GtkExecutorError::ChildIndexOutOfRange {
                    parent: parent.clone(),
                    index: start,
                    child_count,
                });
            }
            let mut remaining = parent_state.children.clone();
            let block = remaining.drain(start..start + count).collect::<Vec<_>>();
            if to > remaining.len() {
                return Err(GtkExecutorError::ChildIndexOutOfRange {
                    parent: parent.clone(),
                    index: to,
                    child_count: remaining.len(),
                });
            }
            let block_widgets = self.root_widgets_for_block(&block)?;
            let local_from = self.root_widget_offset(&parent_state.children, start)?;
            let local_to = self.root_widget_offset(&remaining, to)?;
            (child_count, block_widgets, local_from, local_to)
        };
        debug_assert!(start <= child_count);
        if let Some(container) = self.widget_container_ancestor(parent)? {
            if !block_widgets.is_empty() && local_from != local_to {
                let abs_from = self.absolute_offset_in_ancestor(parent, local_from)?;
                let abs_to = self.absolute_offset_in_ancestor(parent, local_to)?;
                let widget = self.widget_handle(&container)?.clone();
                self.host
                    .move_children(
                        &widget,
                        abs_from,
                        block_widgets.len(),
                        abs_to,
                        &block_widgets,
                    )
                    .map_err(GtkExecutorError::Host)?;
            }
        }
        let block = self
            .instance_state_mut(parent)?
            .children
            .drain(start..start + count)
            .collect::<Vec<_>>();
        for (offset, child) in block.into_iter().enumerate() {
            self.instance_state_mut(parent)?
                .children
                .insert(to + offset, child);
        }
        self.refresh_root_widgets_from(parent.clone())
    }

    fn teardown_subtrees(
        &mut self,
        roots: Vec<GtkNodeInstance>,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let mut stack = roots
            .into_iter()
            .map(|instance| TeardownFrame {
                instance,
                visited: false,
            })
            .collect::<Vec<_>>();
        while let Some(frame) = stack.pop() {
            if frame.visited {
                let state = self.instances.remove(&frame.instance).ok_or(
                    GtkExecutorError::MissingInstance {
                        instance: frame.instance.clone(),
                    },
                )?;
                if let MountedNodeKind::Widget(widget) = state.kind {
                    for route_id in widget.event_routes {
                        let route = self
                            .routes
                            .remove(&route_id)
                            .ok_or(GtkExecutorError::MissingEventRoute { route: route_id })?;
                        self.host
                            .disconnect_event(&route.widget, &route.handle)
                            .map_err(GtkExecutorError::Host)?;
                    }
                    self.host
                        .release_widget(widget.handle)
                        .map_err(GtkExecutorError::Host)?;
                }
                continue;
            }
            let children = self.instance_state(&frame.instance)?.children.clone();
            stack.push(TeardownFrame {
                instance: frame.instance,
                visited: true,
            });
            for child in children.into_iter().rev() {
                stack.push(TeardownFrame {
                    instance: child,
                    visited: false,
                });
            }
        }
        Ok(())
    }

    fn refresh_root_widgets_from(
        &mut self,
        start: GtkNodeInstance,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let mut current = Some(start);
        while let Some(instance) = current {
            let (parent, roots) = {
                let state = self.instance_state(&instance)?;
                let parent = state.parent.clone();
                let roots = match &state.kind {
                    MountedNodeKind::Widget(widget) => vec![widget.handle.clone()],
                    _ => self.root_widgets_for_block(&state.children)?,
                };
                (parent, roots)
            };
            self.instance_state_mut(&instance)?.root_widgets = roots;
            current = parent;
        }
        Ok(())
    }

    fn widget_container_ancestor(
        &self,
        start: &GtkNodeInstance,
    ) -> Result<Option<GtkNodeInstance>, GtkExecutorError<H::Error>> {
        let mut current = Some(start.clone());
        while let Some(instance) = current {
            let state = self.instance_state(&instance)?;
            if matches!(state.kind, MountedNodeKind::Widget(_)) {
                return Ok(Some(instance));
            }
            current = state.parent.clone();
        }
        Ok(None)
    }

    fn absolute_offset_in_ancestor(
        &self,
        start: &GtkNodeInstance,
        local_offset: usize,
    ) -> Result<usize, GtkExecutorError<H::Error>> {
        let mut current = start.clone();
        let mut offset = local_offset;
        loop {
            let state = self.instance_state(&current)?;
            if matches!(state.kind, MountedNodeKind::Widget(_)) {
                return Ok(offset);
            }
            let parent = state
                .parent
                .clone()
                .ok_or(GtkExecutorError::MissingInstance {
                    instance: start.clone(),
                })?;
            let sibling_index = self.find_child_index(&parent, &current)?;
            offset +=
                self.root_widget_offset(&self.instance_state(&parent)?.children, sibling_index)?;
            current = parent;
        }
    }

    fn next_event_route_id(&mut self) -> GtkEventRouteId {
        let id = GtkEventRouteId(self.next_route);
        self.next_route = self
            .next_route
            .checked_add(1)
            .expect("GTK event route counter overflow");
        id
    }

    fn root_widget_offset(
        &self,
        children: &[GtkNodeInstance],
        end: usize,
    ) -> Result<usize, GtkExecutorError<H::Error>> {
        let end = end.min(children.len());
        let mut offset = 0;
        for child in &children[..end] {
            offset += self.instance_state(child)?.root_widgets.len();
        }
        Ok(offset)
    }

    fn root_widgets_for_block(
        &self,
        block: &[GtkNodeInstance],
    ) -> Result<Vec<H::Widget>, GtkExecutorError<H::Error>> {
        let mut widgets = Vec::new();
        for instance in block {
            widgets.extend(self.instance_root_widgets(instance)?);
        }
        Ok(widgets)
    }

    fn instance_root_widgets(
        &self,
        instance: &GtkNodeInstance,
    ) -> Result<Vec<H::Widget>, GtkExecutorError<H::Error>> {
        Ok(self.instance_state(instance)?.root_widgets.clone())
    }

    fn find_child_index(
        &self,
        parent: &GtkNodeInstance,
        child: &GtkNodeInstance,
    ) -> Result<usize, GtkExecutorError<H::Error>> {
        self.find_child_index_opt(parent, child)?
            .ok_or(GtkExecutorError::ChildMissing {
                parent: parent.clone(),
                child: child.clone(),
            })
    }

    fn find_child_index_opt(
        &self,
        parent: &GtkNodeInstance,
        child: &GtkNodeInstance,
    ) -> Result<Option<usize>, GtkExecutorError<H::Error>> {
        Ok(self
            .instance_state(parent)?
            .children
            .iter()
            .position(|candidate| candidate == child))
    }

    fn expect_node_kind(
        &self,
        instance: &GtkNodeInstance,
        expected: PlanNodeTag,
    ) -> Result<(), GtkExecutorError<H::Error>> {
        let found = self.bridge_tag(instance.node)?;
        if found == expected {
            Ok(())
        } else {
            Err(GtkExecutorError::UnexpectedNodeKind {
                instance: instance.clone(),
                expected,
                found,
            })
        }
    }

    fn bridge_tag(
        &self,
        node: GtkBridgeNodeRef,
    ) -> Result<PlanNodeTag, GtkExecutorError<H::Error>> {
        Ok(self.bridge_node(node)?.kind.tag())
    }

    fn bridge_node(
        &self,
        node: GtkBridgeNodeRef,
    ) -> Result<&crate::GtkBridgeNode, GtkExecutorError<H::Error>> {
        let bridge_node = self.bridge.node(node.plan).ok_or(GtkExecutorError::Bridge(
            GtkBridgeExecutionError::MissingNode { node },
        ))?;
        if bridge_node.owner != node.owner {
            return Err(GtkExecutorError::Bridge(
                GtkBridgeExecutionError::NodeOwnerMismatch {
                    node,
                    recorded: bridge_node.owner,
                },
            ));
        }
        Ok(bridge_node)
    }

    fn instance_state(
        &self,
        instance: &GtkNodeInstance,
    ) -> Result<&MountedNode<H::Widget>, GtkExecutorError<H::Error>> {
        self.instances
            .get(instance)
            .ok_or(GtkExecutorError::MissingInstance {
                instance: instance.clone(),
            })
    }

    fn instance_state_mut(
        &mut self,
        instance: &GtkNodeInstance,
    ) -> Result<&mut MountedNode<H::Widget>, GtkExecutorError<H::Error>> {
        self.instances
            .get_mut(instance)
            .ok_or(GtkExecutorError::MissingInstance {
                instance: instance.clone(),
            })
    }
}

#[derive(Clone, Debug)]
struct GtkSetterSite {
    node: GtkBridgeNodeRef,
    binding: RuntimeSetterBinding,
}

#[derive(Clone, Debug)]
struct MountedRoute<W, E> {
    route: GtkEventRoute,
    widget: W,
    handle: E,
}

#[derive(Clone, Debug)]
struct MountedNode<W> {
    parent: Option<GtkNodeInstance>,
    children: Vec<GtkNodeInstance>,
    root_widgets: Vec<W>,
    kind: MountedNodeKind<W>,
}

#[derive(Clone, Debug)]
enum MountedNodeKind<W> {
    Widget(MountedWidget<W>),
    Structural,
    Show(MountedShow),
    Match(MountedMatch),
    Each(MountedEach),
}

#[derive(Clone, Debug)]
struct MountedWidget<W> {
    handle: W,
    event_routes: Vec<GtkEventRouteId>,
}

#[derive(Clone, Debug)]
struct MountedShow {
    state: GtkShowState,
}

#[derive(Clone, Debug)]
struct MountedMatch {
    active_case: Option<usize>,
    active_case_instance: Option<GtkNodeInstance>,
}

#[derive(Clone, Debug)]
struct MountedEach {
    initialized: bool,
    empty_branch: Option<GtkNodeInstance>,
    items: Vec<MountedEachItem>,
}

#[derive(Clone, Debug)]
struct MountedEachItem {
    identity: GtkRepeatedChildIdentity,
    roots: Vec<GtkNodeInstance>,
}

#[derive(Clone, Debug)]
struct MountFrame {
    instance: GtkNodeInstance,
    phase: MountPhase,
}

impl MountFrame {
    fn enter(instance: GtkNodeInstance) -> Self {
        Self {
            instance,
            phase: MountPhase::Enter,
        }
    }

    fn exit(instance: GtkNodeInstance, fixed_children: Vec<GtkBridgeNodeRef>) -> Self {
        Self {
            instance,
            phase: MountPhase::Exit { fixed_children },
        }
    }
}

#[derive(Clone, Debug)]
enum MountPhase {
    Enter,
    Exit {
        fixed_children: Vec<GtkBridgeNodeRef>,
    },
}

#[derive(Clone, Debug)]
struct TeardownFrame {
    instance: GtkNodeInstance,
    visited: bool,
}

fn collect_setter_sites(graph: &GtkBridgeGraph) -> BTreeMap<InputHandle, GtkSetterSite> {
    let mut sites = BTreeMap::new();
    for node in graph.nodes() {
        let Some(node_ref) = graph.node_ref(node.plan) else {
            continue;
        };
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        for property in widget.properties.iter() {
            let RuntimePropertyBinding::Setter(binding) = property else {
                continue;
            };
            sites.insert(
                binding.input,
                GtkSetterSite {
                    node: node_ref,
                    binding: binding.clone(),
                },
            );
        }
    }
    sites
}

fn fixed_children(kind: &GtkBridgeNodeKind) -> Vec<GtkBridgeNodeRef> {
    match kind {
        GtkBridgeNodeKind::Widget(widget) => widget.default_children.roots.to_vec(),
        GtkBridgeNodeKind::Show(_) => Vec::new(),
        GtkBridgeNodeKind::Each(_) => Vec::new(),
        GtkBridgeNodeKind::Empty(empty) => empty.body.roots.to_vec(),
        GtkBridgeNodeKind::Match(_) => Vec::new(),
        GtkBridgeNodeKind::Case(case) => case.body.roots.to_vec(),
        GtkBridgeNodeKind::Fragment(fragment) => fragment.body.roots.to_vec(),
        GtkBridgeNodeKind::With(with_node) => with_node.body.roots.to_vec(),
    }
}

impl fmt::Display for GtkRepeatedChildIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Positional(index) => write!(f, "pos:{index}"),
            Self::Keyed(key) => write!(f, "key:{key}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, TextSegment, lower_module};
    use aivi_syntax::parse_module;

    use super::*;
    use crate::{RuntimePropertyBinding, lower_markup_expr, lower_widget_bridge};

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct TestWidget(u32);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestEventHandle(u32);

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestValue {
        Bool(bool),
        Text(String),
        Unit,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct WidgetRecord {
        kind: String,
        visible: bool,
        children: Vec<TestWidget>,
        static_props: BTreeMap<String, String>,
        dynamic_props: BTreeMap<String, TestValue>,
        routes: Vec<GtkEventRouteId>,
    }

    #[derive(Default)]
    struct TestHost {
        next_widget: u32,
        next_event: u32,
        widgets: BTreeMap<u32, WidgetRecord>,
        released: Vec<TestWidget>,
        insert_ops: Vec<(TestWidget, usize, Vec<TestWidget>)>,
        remove_ops: Vec<(TestWidget, usize, Vec<TestWidget>)>,
        move_ops: Vec<(TestWidget, usize, usize, usize, Vec<TestWidget>)>,
        visibility_ops: Vec<(TestWidget, bool)>,
        disconnected: Vec<GtkEventRouteId>,
    }

    impl TestHost {
        fn widget(&self, widget: &TestWidget) -> &WidgetRecord {
            self.widgets
                .get(&widget.0)
                .unwrap_or_else(|| panic!("missing widget {}", widget.0))
        }

        fn child_ids(&self, widget: &TestWidget) -> Vec<u32> {
            self.widget(widget)
                .children
                .iter()
                .map(|child| child.0)
                .collect()
        }
    }

    impl GtkRuntimeHost<TestValue> for TestHost {
        type Widget = TestWidget;
        type EventHandle = TestEventHandle;
        type Error = Infallible;

        fn create_widget(
            &mut self,
            _instance: &GtkNodeInstance,
            widget: &aivi_hir::NamePath,
        ) -> Result<Self::Widget, Self::Error> {
            let handle = TestWidget(self.next_widget);
            self.next_widget += 1;
            self.widgets.insert(
                handle.0,
                WidgetRecord {
                    kind: widget.to_string(),
                    visible: true,
                    ..WidgetRecord::default()
                },
            );
            Ok(handle)
        }

        fn apply_static_property(
            &mut self,
            widget: &Self::Widget,
            property: &StaticPropertyPlan,
        ) -> Result<(), Self::Error> {
            self.widgets
                .get_mut(&widget.0)
                .expect("static property target should exist")
                .static_props
                .insert(
                    property.name.text().to_string(),
                    static_property_value(property),
                );
            Ok(())
        }

        fn apply_dynamic_property(
            &mut self,
            widget: &Self::Widget,
            binding: &RuntimeSetterBinding,
            value: &TestValue,
        ) -> Result<(), Self::Error> {
            self.widgets
                .get_mut(&widget.0)
                .expect("dynamic property target should exist")
                .dynamic_props
                .insert(binding.name.text().to_string(), value.clone());
            Ok(())
        }

        fn connect_event(
            &mut self,
            widget: &Self::Widget,
            route: &GtkEventRoute,
        ) -> Result<Self::EventHandle, Self::Error> {
            let handle = TestEventHandle(self.next_event);
            self.next_event += 1;
            self.widgets
                .get_mut(&widget.0)
                .expect("event target should exist")
                .routes
                .push(route.id);
            Ok(handle)
        }

        fn disconnect_event(
            &mut self,
            widget: &Self::Widget,
            _event: &Self::EventHandle,
        ) -> Result<(), Self::Error> {
            if let Some(route) = self
                .widgets
                .get_mut(&widget.0)
                .expect("disconnect target should exist")
                .routes
                .pop()
            {
                self.disconnected.push(route);
            }
            Ok(())
        }

        fn insert_children(
            &mut self,
            parent: &Self::Widget,
            index: usize,
            children: &[Self::Widget],
        ) -> Result<(), Self::Error> {
            let record = self
                .widgets
                .get_mut(&parent.0)
                .expect("parent should exist");
            for (offset, child) in children.iter().cloned().enumerate() {
                record.children.insert(index + offset, child);
            }
            self.insert_ops
                .push((parent.clone(), index, children.to_vec()));
            Ok(())
        }

        fn remove_children(
            &mut self,
            parent: &Self::Widget,
            index: usize,
            children: &[Self::Widget],
        ) -> Result<(), Self::Error> {
            let record = self
                .widgets
                .get_mut(&parent.0)
                .expect("parent should exist");
            assert_eq!(&record.children[index..index + children.len()], children);
            record.children.drain(index..index + children.len());
            self.remove_ops
                .push((parent.clone(), index, children.to_vec()));
            Ok(())
        }

        fn move_children(
            &mut self,
            parent: &Self::Widget,
            from: usize,
            count: usize,
            to: usize,
            children: &[Self::Widget],
        ) -> Result<(), Self::Error> {
            let record = self
                .widgets
                .get_mut(&parent.0)
                .expect("parent should exist");
            let moved = record
                .children
                .drain(from..from + count)
                .collect::<Vec<_>>();
            assert_eq!(moved, children);
            for (offset, child) in moved.iter().cloned().enumerate() {
                record.children.insert(to + offset, child);
            }
            self.move_ops
                .push((parent.clone(), from, count, to, children.to_vec()));
            Ok(())
        }

        fn set_widget_visibility(
            &mut self,
            widget: &Self::Widget,
            visible: bool,
        ) -> Result<(), Self::Error> {
            self.widgets
                .get_mut(&widget.0)
                .expect("visibility target should exist")
                .visible = visible;
            self.visibility_ops.push((widget.clone(), visible));
            Ok(())
        }

        fn release_widget(&mut self, widget: Self::Widget) -> Result<(), Self::Error> {
            self.released.push(widget);
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestSink {
        dispatched: Vec<(GtkEventRouteId, GtkNodeInstance, u32, TestValue)>,
    }

    impl GtkEventSink<TestValue> for TestSink {
        type Error = Infallible;

        fn dispatch_event(
            &mut self,
            route: &GtkEventRoute,
            value: TestValue,
        ) -> Result<(), Self::Error> {
            self.dispatched.push((
                route.id,
                route.instance.clone(),
                route.binding.input.as_raw(),
                value,
            ));
            Ok(())
        }
    }

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

    fn lower_graph(path: &str, text: &str) -> GtkBridgeGraph {
        let hir = lower_text(path, text);
        assert!(
            !hir.has_errors(),
            "fixture {path} should lower cleanly: {:?}",
            hir.diagnostics()
        );
        let module = hir.module();
        let value = find_value_item(module, "view");
        let plan = lower_markup_expr(module, value.body).expect("markup should lower");
        lower_widget_bridge(&plan).expect("GTK bridge graph should build")
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
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
            "fixture should lower cleanly: {:?}",
            hir.diagnostics()
        );
        let module = hir.module();
        let value = find_value_item(module, "screenView");
        let plan = lower_markup_expr(module, value.body).expect("fixture markup should lower");
        lower_widget_bridge(&plan).expect("fixture should lower into a GTK bridge graph")
    }

    fn static_property_value(property: &StaticPropertyPlan) -> String {
        match &property.value {
            crate::StaticPropertyValue::ImplicitTrue => "true".to_string(),
            crate::StaticPropertyValue::Text(text) => text
                .segments
                .iter()
                .filter_map(|segment| match segment {
                    TextSegment::Text(fragment) => Some(fragment.raw.as_ref()),
                    TextSegment::Interpolation(_) => None,
                })
                .collect::<String>(),
        }
    }

    fn find_button_show_refs(
        graph: &GtkBridgeGraph,
    ) -> (GtkNodeInstance, InputHandle, GtkNodeInstance) {
        let root = graph.root_node();
        let GtkBridgeNodeKind::Widget(root_widget) = &root.kind else {
            panic!("expected root widget, found {:?}", root.kind.tag());
        };
        let show_ref = root_widget.default_children.roots[0];
        let show = graph.node(show_ref.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_node) = &show.kind else {
            panic!("expected show node, found {:?}", show.kind.tag());
        };
        let button_ref = show_node.body.roots[0];
        let button = graph
            .node(button_ref.plan)
            .expect("button node should exist");
        let GtkBridgeNodeKind::Widget(button_node) = &button.kind else {
            panic!("expected button widget, found {:?}", button.kind.tag());
        };
        let visible = button_node
            .properties
            .iter()
            .find_map(|property| match property {
                RuntimePropertyBinding::Setter(binding) if binding.name.text() == "visible" => {
                    Some(binding.input)
                }
                _ => None,
            })
            .expect("button should keep a visible setter");
        (
            GtkNodeInstance::root(show_ref),
            visible,
            GtkNodeInstance::root(button_ref),
        )
    }

    fn find_box_each_ref(graph: &GtkBridgeGraph) -> (GtkNodeInstance, TestWidget) {
        let root_widget = TestWidget(0);
        let root = graph.root_node();
        let GtkBridgeNodeKind::Widget(widget) = &root.kind else {
            panic!("expected root widget, found {:?}", root.kind.tag());
        };
        (
            GtkNodeInstance::root(widget.default_children.roots[0]),
            root_widget,
        )
    }

    #[test]
    fn executor_mounts_widgets_replays_cached_setters_and_dispatches_events() {
        let graph = lower_graph(
            "show-button.aivi",
            r#"
val keep = False
val isVisible = True
val click = True
val view =
    <Box>
        <show when={True} keepMounted={keep}>
            <Button label="Save" visible={isVisible} onClick={click} />
        </show>
    </Box>
"#,
        );
        let (show, visible_input, button_instance) = find_button_show_refs(&graph);
        let mut executor = GtkRuntimeExecutor::new_with_values(
            graph,
            TestHost::default(),
            [(visible_input, TestValue::Bool(false))],
        )
        .expect("executor should mount the static root subtree");
        let root_widget = executor
            .root_widgets()
            .expect("root widget should exist")
            .into_iter()
            .next()
            .expect("root widget list should contain the box");
        assert_eq!(executor.host().widget(&root_widget).kind, "Box");
        assert!(executor.host().child_ids(&root_widget).is_empty());

        executor
            .update_show(&show, false, false)
            .expect("keepMounted=False should leave the subtree unmounted");
        assert!(executor.host().child_ids(&root_widget).is_empty());

        executor
            .update_show(&show, true, false)
            .expect("show should mount the button subtree");
        assert!(executor.is_mounted(&button_instance));
        let button = executor
            .widget_handle(&button_instance)
            .expect("button instance should now be mounted")
            .clone();
        assert_eq!(executor.host().widget(&button).kind, "Button");
        assert_eq!(
            executor.host().widget(&button).static_props.get("label"),
            Some(&"Save".to_string())
        );
        assert_eq!(
            executor.host().widget(&button).dynamic_props.get("visible"),
            Some(&TestValue::Bool(false))
        );
        assert_eq!(executor.host().child_ids(&root_widget), vec![button.0]);

        let routes = executor.event_routes_for_instance(&button_instance);
        assert_eq!(routes.len(), 1);
        let route = routes[0].clone();
        let mut sink = TestSink::default();
        executor
            .dispatch_event(route.id, TestValue::Unit, &mut sink)
            .expect("dispatch should hand the payload to the sink");
        assert_eq!(
            sink.dispatched,
            vec![(
                route.id,
                button_instance.clone(),
                route.binding.input.as_raw(),
                TestValue::Unit,
            )]
        );

        executor
            .update_show(&show, false, false)
            .expect("show hide should teardown the button subtree");
        assert!(!executor.is_mounted(&button_instance));
        assert!(executor.host().child_ids(&root_widget).is_empty());
        assert_eq!(executor.host().released, vec![button.clone()]);
        assert_eq!(executor.host().disconnected, vec![route.id]);
    }

    #[test]
    fn executor_drives_keyed_each_child_management_and_empty_branch_mounts() {
        let graph = lower_graph(
            "keyed-each.aivi",
            r#"
val items = [1]
val view =
    <Box>
        <each of={items} as={item} key={item}>
            <Label text="Row" />
            <empty>
                <Label text="Empty" />
            </empty>
        </each>
    </Box>
"#,
        );
        let (each_instance, root_widget) = find_box_each_ref(&graph);
        let mut executor =
            GtkRuntimeExecutor::<TestHost, TestValue>::new(graph, TestHost::default())
                .expect("executor should mount the root box");

        executor
            .update_each_keyed(&each_instance, &[])
            .expect("first empty keyed update should mount the empty branch");
        let empty_widget = executor.host().widget(&TestWidget(1)).clone();
        assert_eq!(empty_widget.kind, "Label");
        assert_eq!(
            executor
                .host()
                .widget(&TestWidget(1))
                .static_props
                .get("text"),
            Some(&"Empty".to_string())
        );
        assert_eq!(executor.host().child_ids(&root_widget), vec![1]);

        let next = [GtkCollectionKey::from("a"), GtkCollectionKey::from("b")];
        executor
            .update_each_keyed(&each_instance, &next)
            .expect("keyed insertion should replace the empty branch with rows");
        assert_eq!(executor.host().child_ids(&root_widget), vec![2, 3]);
        assert_eq!(executor.host().released, vec![TestWidget(1)]);

        let reordered = [GtkCollectionKey::from("b"), GtkCollectionKey::from("a")];
        executor
            .update_each_keyed(&each_instance, &reordered)
            .expect("keyed move should reuse the mounted row widgets");
        assert_eq!(executor.host().child_ids(&root_widget), vec![3, 2]);
        assert_eq!(executor.host().move_ops.len(), 1);
        assert_eq!(executor.host().released, vec![TestWidget(1)]);

        executor
            .update_each_keyed(&each_instance, &[])
            .expect("draining the keyed collection should restore the empty branch");
        assert_eq!(executor.host().child_ids(&root_widget), vec![4]);
        assert_eq!(
            executor.host().released,
            vec![TestWidget(1), TestWidget(2), TestWidget(3)]
        );
    }

    #[test]
    fn executor_switches_match_cases_by_mounting_case_subtrees_directly() {
        let graph = control_fixture_graph();
        let root = graph.root_node();
        let GtkBridgeNodeKind::Fragment(fragment) = &root.kind else {
            panic!("expected fragment root, found {:?}", root.kind.tag());
        };
        let show = GtkNodeInstance::root(fragment.body.roots[1]);
        let show_node = graph.node(show.node.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_data) = &show_node.kind else {
            panic!("expected show node, found {:?}", show_node.kind.tag());
        };
        let with_ref = show_data.body.roots[0];
        let with_node = graph.node(with_ref.plan).expect("with node should exist");
        let GtkBridgeNodeKind::With(with_node) = &with_node.kind else {
            panic!("expected with node, found {:?}", with_node.kind.tag());
        };
        let match_instance = GtkNodeInstance::root(with_node.body.roots[0]);

        let mut executor =
            GtkRuntimeExecutor::<TestHost, TestValue>::new(graph, TestHost::default())
                .expect("executor should mount the fragment header");
        assert_eq!(
            executor
                .root_widgets()
                .expect("fragment roots should be materialized")
                .iter()
                .map(|widget| widget.0)
                .collect::<Vec<_>>(),
            vec![0],
        );

        executor
            .update_show(&show, true, true)
            .expect("show should mount the with/match scaffolding");
        executor
            .update_match(&match_instance, 0)
            .expect("match should mount the loading case");
        assert_eq!(
            executor
                .root_widgets()
                .expect("fragment roots should update")
                .iter()
                .map(|widget| widget.0)
                .collect::<Vec<_>>(),
            vec![0, 1],
        );
        assert_eq!(
            executor
                .host()
                .widget(&TestWidget(1))
                .static_props
                .get("text"),
            Some(&"Loading...".to_string())
        );

        executor
            .update_match(&match_instance, 2)
            .expect("switching cases should teardown the old subtree and mount the new one");
        assert_eq!(
            executor
                .root_widgets()
                .expect("fragment roots should update")
                .iter()
                .map(|widget| widget.0)
                .collect::<Vec<_>>(),
            vec![0, 2],
        );
        assert_eq!(executor.host().released, vec![TestWidget(1)]);
    }

    #[test]
    fn executor_applies_shared_setter_updates_to_all_live_template_instances() {
        let graph = control_fixture_graph();
        let root = graph.root_node();
        let GtkBridgeNodeKind::Fragment(fragment) = &root.kind else {
            panic!("expected fragment root, found {:?}", root.kind.tag());
        };
        let show = GtkNodeInstance::root(fragment.body.roots[1]);
        let show_node = graph.node(show.node.plan).expect("show node should exist");
        let GtkBridgeNodeKind::Show(show_data) = &show_node.kind else {
            panic!("expected show node, found {:?}", show_node.kind.tag());
        };
        let with_ref = show_data.body.roots[0];
        let with_node = graph.node(with_ref.plan).expect("with node should exist");
        let GtkBridgeNodeKind::With(with_node) = &with_node.kind else {
            panic!("expected with node, found {:?}", with_node.kind.tag());
        };
        let match_instance = GtkNodeInstance::root(with_node.body.roots[0]);
        let match_node = graph
            .node(match_instance.node.plan)
            .expect("match node should exist");
        let GtkBridgeNodeKind::Match(match_data) = &match_node.kind else {
            panic!("expected match node, found {:?}", match_node.kind.tag());
        };
        let each_instance = GtkNodeInstance::root(match_data.cases[1].body.roots[0]);
        let each_node = graph
            .node(each_instance.node.plan)
            .expect("each node should exist");
        let GtkBridgeNodeKind::Each(each_data) = &each_node.kind else {
            panic!("expected each node, found {:?}", each_node.kind.tag());
        };
        let row_ref = each_data.item_template.roots[0];
        let row_node = graph.node(row_ref.plan).expect("row node should exist");
        let GtkBridgeNodeKind::Widget(row_data) = &row_node.kind else {
            panic!("expected row widget, found {:?}", row_node.kind.tag());
        };
        let title_input = row_data
            .properties
            .iter()
            .find_map(|property| match property {
                RuntimePropertyBinding::Setter(binding) if binding.name.text() == "title" => {
                    Some(binding.input)
                }
                _ => None,
            })
            .expect("row template should carry the title setter");

        let mut executor =
            GtkRuntimeExecutor::<TestHost, TestValue>::new(graph, TestHost::default())
                .expect("executor should mount the fragment header");
        executor.update_show(&show, true, true).unwrap();
        executor.update_match(&match_instance, 1).unwrap();
        executor
            .update_each_keyed(
                &each_instance,
                &[
                    GtkCollectionKey::from("alpha"),
                    GtkCollectionKey::from("beta"),
                ],
            )
            .unwrap();

        executor
            .set_property(title_input, TestValue::Text("Shared title".to_string()))
            .expect("shared setter should update every live row instance");
        assert_eq!(
            executor
                .host()
                .widget(&TestWidget(1))
                .dynamic_props
                .get("title"),
            Some(&TestValue::Text("Shared title".to_string()))
        );
        assert_eq!(
            executor
                .host()
                .widget(&TestWidget(2))
                .dynamic_props
                .get("title"),
            Some(&TestValue::Text("Shared title".to_string()))
        );
    }
}
