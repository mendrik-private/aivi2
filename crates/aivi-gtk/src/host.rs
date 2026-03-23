use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    rc::Rc,
};

use aivi_hir::{NamePath, TextLiteral, TextSegment};
use gtk::{Orientation, glib::SignalHandlerId, prelude::*};

use crate::{
    GtkEventRoute, GtkEventRouteId, GtkRuntimeHost, RuntimeSetterBinding, StaticPropertyPlan,
    StaticPropertyValue,
};

pub trait GtkHostValue: Clone + 'static {
    fn unit() -> Self;

    fn as_bool(&self) -> Option<bool> {
        None
    }

    fn as_i64(&self) -> Option<i64> {
        None
    }

    fn as_text(&self) -> Option<&str> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkConcreteWidget(u64);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GtkConcreteEventHandle(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkQueuedEvent<V> {
    pub route: GtkEventRouteId,
    pub value: V,
}

pub struct GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    next_widget: u64,
    next_event: u64,
    widgets: BTreeMap<u64, MountedWidget>,
    events: BTreeMap<u64, MountedEvent>,
    queued_events: Rc<RefCell<VecDeque<GtkQueuedEvent<V>>>>,
}

impl<V> Default for GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    fn default() -> Self {
        Self {
            next_widget: 0,
            next_event: 0,
            widgets: BTreeMap::new(),
            events: BTreeMap::new(),
            queued_events: Rc::new(RefCell::new(VecDeque::new())),
        }
    }
}

impl<V> GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    pub fn widget(&self, handle: &GtkConcreteWidget) -> Option<gtk::Widget> {
        self.widgets
            .get(&handle.0)
            .map(|mounted| mounted.widget.clone())
    }

    pub fn child_handles(&self, handle: &GtkConcreteWidget) -> Option<Vec<GtkConcreteWidget>> {
        self.widgets
            .get(&handle.0)
            .map(|mounted| mounted.children.clone())
    }

    pub fn drain_events(&mut self) -> Vec<GtkQueuedEvent<V>> {
        self.queued_events.borrow_mut().drain(..).collect()
    }

    pub fn present_root_windows(&self) {
        for mounted in self.widgets.values() {
            if mounted.kind == SupportedWidget::Window && mounted.widget.parent().is_none() {
                let window = mounted
                    .widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .expect("window handles should downcast cleanly");
                window.present();
            }
        }
    }

    fn create_supported_widget(
        &self,
        widget: &NamePath,
    ) -> Result<(SupportedWidget, gtk::Widget), GtkConcreteHostError> {
        let name = widget_name(widget);
        let (kind, widget) = match name {
            "Window" => (
                SupportedWidget::Window,
                gtk::Window::new().upcast::<gtk::Widget>(),
            ),
            "Box" => (
                SupportedWidget::Box,
                gtk::Box::new(Orientation::Vertical, 0).upcast::<gtk::Widget>(),
            ),
            "Label" => (
                SupportedWidget::Label,
                gtk::Label::new(None).upcast::<gtk::Widget>(),
            ),
            "Button" => (
                SupportedWidget::Button,
                gtk::Button::new().upcast::<gtk::Widget>(),
            ),
            other => {
                return Err(GtkConcreteHostError::UnsupportedWidget {
                    widget: other.to_owned().into_boxed_str(),
                });
            }
        };
        Ok((kind, widget))
    }

    fn mounted_snapshot(
        &self,
        handle: &GtkConcreteWidget,
    ) -> Result<(SupportedWidget, gtk::Widget, Vec<GtkConcreteWidget>), GtkConcreteHostError> {
        let mounted =
            self.widgets
                .get(&handle.0)
                .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                    widget: handle.clone(),
                })?;
        Ok((
            mounted.kind,
            mounted.widget.clone(),
            mounted.children.clone(),
        ))
    }

    fn widget_object(
        &self,
        handle: &GtkConcreteWidget,
    ) -> Result<gtk::Widget, GtkConcreteHostError> {
        self.widgets
            .get(&handle.0)
            .map(|mounted| mounted.widget.clone())
            .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                widget: handle.clone(),
            })
    }

    fn update_children(
        &mut self,
        handle: &GtkConcreteWidget,
        children: Vec<GtkConcreteWidget>,
    ) -> Result<(), GtkConcreteHostError> {
        let mounted =
            self.widgets
                .get_mut(&handle.0)
                .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                    widget: handle.clone(),
                })?;
        mounted.children = children;
        Ok(())
    }

    fn apply_bool_property(
        &self,
        widget: &gtk::Widget,
        kind: SupportedWidget,
        property: &str,
        value: bool,
    ) -> Result<(), GtkConcreteHostError> {
        match property {
            "visible" => widget.set_visible(value),
            "sensitive" => widget.set_sensitive(value),
            "hexpand" => widget.set_hexpand(value),
            "vexpand" => widget.set_vexpand(value),
            _ => {
                return Err(GtkConcreteHostError::UnsupportedProperty {
                    widget: kind.label().into(),
                    property: property.to_owned().into_boxed_str(),
                });
            }
        }
        Ok(())
    }

    fn apply_text_property(
        &self,
        widget: &gtk::Widget,
        kind: SupportedWidget,
        property: &str,
        value: &str,
    ) -> Result<(), GtkConcreteHostError> {
        match (kind, property) {
            (SupportedWidget::Window, "title") => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .expect("window widget should downcast")
                    .set_title(Some(value));
            }
            (SupportedWidget::Label, "text") => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .expect("label widget should downcast")
                    .set_text(value);
            }
            (SupportedWidget::Label, "label") => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .expect("label widget should downcast")
                    .set_label(value);
            }
            (SupportedWidget::Button, "label") => {
                widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .expect("button widget should downcast")
                    .set_label(value);
            }
            (SupportedWidget::Box, "orientation") => {
                let orientation = parse_orientation(value).ok_or_else(|| {
                    GtkConcreteHostError::InvalidPropertyValue {
                        widget: kind.label().into(),
                        property: property.to_owned().into_boxed_str(),
                        expected: "Vertical or Horizontal",
                    }
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast")
                    .set_orientation(orientation);
            }
            (SupportedWidget::Box, "spacing") => {
                let spacing = value.parse::<i32>().map_err(|_| {
                    GtkConcreteHostError::InvalidPropertyValue {
                        widget: kind.label().into(),
                        property: property.to_owned().into_boxed_str(),
                        expected: "signed 32-bit integer text",
                    }
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast")
                    .set_spacing(spacing);
            }
            _ => {
                return Err(GtkConcreteHostError::UnsupportedProperty {
                    widget: kind.label().into(),
                    property: property.to_owned().into_boxed_str(),
                });
            }
        }
        Ok(())
    }

    fn apply_i64_property(
        &self,
        widget: &gtk::Widget,
        kind: SupportedWidget,
        property: &str,
        value: i64,
    ) -> Result<(), GtkConcreteHostError> {
        match (kind, property) {
            (SupportedWidget::Box, "spacing") => {
                let spacing = i32::try_from(value).map_err(|_| {
                    GtkConcreteHostError::InvalidPropertyValue {
                        widget: kind.label().into(),
                        property: property.to_owned().into_boxed_str(),
                        expected: "signed 32-bit integer",
                    }
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast")
                    .set_spacing(spacing);
                Ok(())
            }
            _ => Err(GtkConcreteHostError::UnsupportedProperty {
                widget: kind.label().into(),
                property: property.to_owned().into_boxed_str(),
            }),
        }
    }
}

impl<V> GtkRuntimeHost<V> for GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    type Widget = GtkConcreteWidget;
    type EventHandle = GtkConcreteEventHandle;
    type Error = GtkConcreteHostError;

    fn create_widget(
        &mut self,
        _instance: &crate::GtkNodeInstance,
        widget: &NamePath,
    ) -> Result<Self::Widget, Self::Error> {
        let handle = GtkConcreteWidget(self.next_widget);
        self.next_widget = self
            .next_widget
            .checked_add(1)
            .expect("concrete GTK widget handle counter should not overflow");
        let (kind, widget) = self.create_supported_widget(widget)?;
        self.widgets.insert(
            handle.0,
            MountedWidget {
                kind,
                widget,
                children: Vec::new(),
            },
        );
        Ok(handle)
    }

    fn apply_static_property(
        &mut self,
        widget: &Self::Widget,
        property: &StaticPropertyPlan,
    ) -> Result<(), Self::Error> {
        let (kind, widget, _) = self.mounted_snapshot(widget)?;
        match &property.value {
            StaticPropertyValue::ImplicitTrue => {
                self.apply_bool_property(&widget, kind, property.name.text(), true)
            }
            StaticPropertyValue::Text(text) => {
                self.apply_text_property(&widget, kind, property.name.text(), &text_literal(text))
            }
        }
    }

    fn apply_dynamic_property(
        &mut self,
        widget: &Self::Widget,
        binding: &RuntimeSetterBinding,
        value: &V,
    ) -> Result<(), Self::Error> {
        let (kind, widget, _) = self.mounted_snapshot(widget)?;
        let property = binding.name.text();
        if let Some(value) = value.as_bool() {
            if matches!(property, "visible" | "sensitive" | "hexpand" | "vexpand") {
                return self.apply_bool_property(&widget, kind, property, value);
            }
        }
        if let Some(value) = value.as_text() {
            return self.apply_text_property(&widget, kind, property, value);
        }
        if let Some(value) = value.as_i64() {
            return self.apply_i64_property(&widget, kind, property, value);
        }
        Err(GtkConcreteHostError::InvalidPropertyValue {
            widget: kind.label().into(),
            property: property.to_owned().into_boxed_str(),
            expected: "a supported GTK host value",
        })
    }

    fn connect_event(
        &mut self,
        widget: &Self::Widget,
        route: &GtkEventRoute,
    ) -> Result<Self::EventHandle, Self::Error> {
        let (kind, widget, _) = self.mounted_snapshot(widget)?;
        let handle = GtkConcreteEventHandle(self.next_event);
        self.next_event = self
            .next_event
            .checked_add(1)
            .expect("concrete GTK event handle counter should not overflow");
        let queue = self.queued_events.clone();
        let route_id = route.id;
        let signal = match (kind, route.binding.name.text()) {
            (SupportedWidget::Button, "onClick") => widget
                .clone()
                .downcast::<gtk::Button>()
                .expect("button widget should downcast")
                .connect_clicked(move |_| {
                    queue.borrow_mut().push_back(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                }),
            _ => {
                return Err(GtkConcreteHostError::UnsupportedEvent {
                    widget: kind.label().into(),
                    event: route.binding.name.text().to_owned().into_boxed_str(),
                });
            }
        };
        self.events.insert(
            handle.0,
            MountedEvent {
                widget: widget.clone(),
                signal,
            },
        );
        Ok(handle)
    }

    fn disconnect_event(
        &mut self,
        _widget: &Self::Widget,
        event: &Self::EventHandle,
    ) -> Result<(), Self::Error> {
        let mounted = self.events.remove(&event.0).ok_or_else(|| {
            GtkConcreteHostError::UnknownEventHandle {
                event: event.clone(),
            }
        })?;
        mounted.widget.disconnect(mounted.signal);
        Ok(())
    }

    fn insert_children(
        &mut self,
        parent: &Self::Widget,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        let (kind, parent_widget, current_children) = self.mounted_snapshot(parent)?;
        if index > current_children.len() {
            return Err(GtkConcreteHostError::ChildIndexOutOfRange {
                parent: parent.clone(),
                index,
                child_count: current_children.len(),
            });
        }
        let child_widgets = children
            .iter()
            .map(|child| self.widget_object(child))
            .collect::<Result<Vec<_>, _>>()?;
        let mut next_children = current_children.clone();
        match kind {
            SupportedWidget::Window => {
                if current_children.len() + children.len() > 1 || index != 0 {
                    return Err(GtkConcreteHostError::UnsupportedParentOperation {
                        parent: parent.clone(),
                        widget: kind.label().into(),
                        operation: "insert_children".into(),
                    });
                }
                let child = child_widgets.first().ok_or_else(|| {
                    GtkConcreteHostError::UnsupportedParentOperation {
                        parent: parent.clone(),
                        widget: kind.label().into(),
                        operation: "insert_children".into(),
                    }
                })?;
                parent_widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .expect("window widget should downcast")
                    .set_child(Some(child));
                next_children.splice(index..index, children.iter().cloned());
            }
            SupportedWidget::Box => {
                let box_widget = parent_widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast");
                let mut insertion_index = index;
                for (child_handle, child_widget) in children.iter().zip(child_widgets.iter()) {
                    let sibling = if insertion_index == 0 {
                        None
                    } else {
                        Some(self.widget_object(&next_children[insertion_index - 1])?)
                    };
                    box_widget.insert_child_after(child_widget, sibling.as_ref());
                    next_children.insert(insertion_index, child_handle.clone());
                    insertion_index += 1;
                }
            }
            _ => {
                return Err(GtkConcreteHostError::UnsupportedParentOperation {
                    parent: parent.clone(),
                    widget: kind.label().into(),
                    operation: "insert_children".into(),
                });
            }
        }
        self.update_children(parent, next_children)
    }

    fn remove_children(
        &mut self,
        parent: &Self::Widget,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        let (kind, parent_widget, current_children) = self.mounted_snapshot(parent)?;
        if index + children.len() > current_children.len() {
            return Err(GtkConcreteHostError::ChildIndexOutOfRange {
                parent: parent.clone(),
                index,
                child_count: current_children.len(),
            });
        }
        if current_children[index..index + children.len()] != *children {
            return Err(GtkConcreteHostError::ChildMismatch {
                parent: parent.clone(),
            });
        }
        let child_widgets = children
            .iter()
            .map(|child| self.widget_object(child))
            .collect::<Result<Vec<_>, _>>()?;
        let mut next_children = current_children.clone();
        match kind {
            SupportedWidget::Window => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .expect("window widget should downcast")
                    .set_child(None::<&gtk::Widget>);
                next_children.clear();
            }
            SupportedWidget::Box => {
                let box_widget = parent_widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast");
                for child in &child_widgets {
                    box_widget.remove(child);
                }
                next_children.drain(index..index + children.len());
            }
            _ => {
                return Err(GtkConcreteHostError::UnsupportedParentOperation {
                    parent: parent.clone(),
                    widget: kind.label().into(),
                    operation: "remove_children".into(),
                });
            }
        }
        self.update_children(parent, next_children)
    }

    fn move_children(
        &mut self,
        parent: &Self::Widget,
        from: usize,
        count: usize,
        to: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        let (kind, parent_widget, current_children) = self.mounted_snapshot(parent)?;
        if from + count > current_children.len()
            || to > current_children.len().saturating_sub(count)
        {
            return Err(GtkConcreteHostError::ChildIndexOutOfRange {
                parent: parent.clone(),
                index: from.max(to),
                child_count: current_children.len(),
            });
        }
        if current_children[from..from + count] != *children {
            return Err(GtkConcreteHostError::ChildMismatch {
                parent: parent.clone(),
            });
        }
        match kind {
            SupportedWidget::Box => {
                let box_widget = parent_widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast");
                let mut next_children = current_children.clone();
                let moved = next_children.drain(from..from + count).collect::<Vec<_>>();
                for (offset, child) in moved.iter().cloned().enumerate() {
                    next_children.insert(to + offset, child);
                }
                for index in 0..next_children.len() {
                    let child_widget = self.widget_object(&next_children[index])?;
                    let sibling = if index == 0 {
                        None
                    } else {
                        Some(self.widget_object(&next_children[index - 1])?)
                    };
                    box_widget.reorder_child_after(&child_widget, sibling.as_ref());
                }
                self.update_children(parent, next_children)
            }
            SupportedWidget::Window if from == 0 && count == 1 && to == 0 => Ok(()),
            _ => Err(GtkConcreteHostError::UnsupportedParentOperation {
                parent: parent.clone(),
                widget: kind.label().into(),
                operation: "move_children".into(),
            }),
        }
    }

    fn set_widget_visibility(
        &mut self,
        widget: &Self::Widget,
        visible: bool,
    ) -> Result<(), Self::Error> {
        let (_, widget, _) = self.mounted_snapshot(widget)?;
        widget.set_visible(visible);
        Ok(())
    }

    fn release_widget(&mut self, widget: Self::Widget) -> Result<(), Self::Error> {
        let mounted = self
            .widgets
            .remove(&widget.0)
            .ok_or(GtkConcreteHostError::UnknownWidget { widget })?;
        let stale_events = self
            .events
            .iter()
            .filter_map(|(id, event)| (event.widget == mounted.widget).then_some(*id))
            .collect::<Vec<_>>();
        for event_id in stale_events {
            if let Some(event) = self.events.remove(&event_id) {
                event.widget.disconnect(event.signal);
            }
        }
        if let Ok(window) = mounted.widget.downcast::<gtk::Window>() {
            window.close();
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupportedWidget {
    Window,
    Box,
    Label,
    Button,
}

impl SupportedWidget {
    const fn label(self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::Box => "Box",
            Self::Label => "Label",
            Self::Button => "Button",
        }
    }
}

struct MountedWidget {
    kind: SupportedWidget,
    widget: gtk::Widget,
    children: Vec<GtkConcreteWidget>,
}

struct MountedEvent {
    widget: gtk::Widget,
    signal: SignalHandlerId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtkConcreteHostError {
    UnknownWidget {
        widget: GtkConcreteWidget,
    },
    UnknownEventHandle {
        event: GtkConcreteEventHandle,
    },
    UnsupportedWidget {
        widget: Box<str>,
    },
    UnsupportedProperty {
        widget: Box<str>,
        property: Box<str>,
    },
    UnsupportedEvent {
        widget: Box<str>,
        event: Box<str>,
    },
    UnsupportedParentOperation {
        parent: GtkConcreteWidget,
        widget: Box<str>,
        operation: Box<str>,
    },
    InvalidPropertyValue {
        widget: Box<str>,
        property: Box<str>,
        expected: &'static str,
    },
    ChildIndexOutOfRange {
        parent: GtkConcreteWidget,
        index: usize,
        child_count: usize,
    },
    ChildMismatch {
        parent: GtkConcreteWidget,
    },
}

impl fmt::Display for GtkConcreteHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWidget { widget } => write!(f, "unknown GTK widget handle {:?}", widget),
            Self::UnknownEventHandle { event } => {
                write!(f, "unknown GTK event handle {:?}", event)
            }
            Self::UnsupportedWidget { widget } => {
                write!(f, "GTK host does not yet support widget `{widget}`")
            }
            Self::UnsupportedProperty { widget, property } => write!(
                f,
                "GTK host does not support property `{property}` on widget `{widget}`"
            ),
            Self::UnsupportedEvent { widget, event } => write!(
                f,
                "GTK host does not support event `{event}` on widget `{widget}`"
            ),
            Self::UnsupportedParentOperation {
                parent,
                widget,
                operation,
            } => write!(
                f,
                "GTK host cannot {operation} for parent {:?} of widget kind `{widget}`",
                parent
            ),
            Self::InvalidPropertyValue {
                widget,
                property,
                expected,
            } => write!(
                f,
                "GTK host expected {expected} for property `{property}` on widget `{widget}`"
            ),
            Self::ChildIndexOutOfRange {
                parent,
                index,
                child_count,
            } => write!(
                f,
                "GTK host parent {:?} requested child index {index}, but only {child_count} child widget(s) exist",
                parent
            ),
            Self::ChildMismatch { parent } => write!(
                f,
                "GTK host parent {:?} was asked to mutate a child range that does not match the mounted order",
                parent
            ),
        }
    }
}

impl Error for GtkConcreteHostError {}

fn widget_name(path: &NamePath) -> &str {
    path.segments()
        .iter()
        .last()
        .expect("NamePath is non-empty")
        .text()
}

fn text_literal(text: &TextLiteral) -> String {
    text.segments
        .iter()
        .filter_map(|segment| match segment {
            TextSegment::Text(fragment) => Some(fragment.raw.as_ref()),
            TextSegment::Interpolation(_) => None,
        })
        .collect()
}

fn parse_orientation(value: &str) -> Option<Orientation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "vertical" => Some(Orientation::Vertical),
        "horizontal" => Some(Orientation::Horizontal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module};
    use aivi_runtime::InputHandle;
    use aivi_syntax::parse_module;
    use gtk::prelude::*;

    use crate::{
        GtkBridgeGraph, GtkBridgeNodeKind, GtkRuntimeExecutor, RuntimePropertyBinding,
        lower_markup_expr, lower_widget_bridge,
    };

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestValue {
        Bool(bool),
        Int(i64),
        Text(String),
        Unit,
    }

    impl GtkHostValue for TestValue {
        fn unit() -> Self {
            Self::Unit
        }

        fn as_bool(&self) -> Option<bool> {
            match self {
                Self::Bool(value) => Some(*value),
                _ => None,
            }
        }

        fn as_i64(&self) -> Option<i64> {
            match self {
                Self::Int(value) => Some(*value),
                _ => None,
            }
        }

        fn as_text(&self) -> Option<&str> {
            match self {
                Self::Text(value) => Some(value),
                _ => None,
            }
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

    fn lower_graph(path: &str, text: &str) -> GtkBridgeGraph {
        let hir = lower_text(path, text);
        assert!(
            !hir.has_errors(),
            "fixture {path} should lower cleanly: {:?}",
            hir.diagnostics()
        );
        let module = hir.module();
        let value = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(value) if value.name.text() == "view" => Some(value),
                _ => None,
            })
            .expect("expected a `view` value item");
        let plan = lower_markup_expr(module, value.body).expect("markup should lower");
        lower_widget_bridge(&plan).expect("GTK bridge graph should build")
    }

    fn find_widget_input(graph: &GtkBridgeGraph, widget_name: &str, property: &str) -> InputHandle {
        graph
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if super::widget_name(&widget.widget) == widget_name =>
                {
                    widget.properties.iter().find_map(|binding| match binding {
                        RuntimePropertyBinding::Setter(binding)
                            if binding.name.text() == property =>
                        {
                            Some(binding.input)
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected {widget_name}.{property} input"))
    }

    #[test]
    fn concrete_host_mounts_widgets_applies_properties_and_captures_clicks() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host.aivi",
                r#"
val titleText = "Runtime title"
val gap = 4
val isVisible = False
val isEnabled = True
val click = True
val view =
    <Window title="Host">
        <Box orientation="Vertical" spacing={gap}>
            <Label text={titleText} />
            <Button label="Save" visible={isVisible} sensitive={isEnabled} onClick={click} />
        </Box>
    </Window>
"#,
            );
            let title_input = find_widget_input(&graph, "Label", "text");
            let spacing_input = find_widget_input(&graph, "Box", "spacing");
            let visible_input = find_widget_input(&graph, "Button", "visible");
            let sensitive_input = find_widget_input(&graph, "Button", "sensitive");
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (title_input, TestValue::Text("Runtime title".to_string())),
                    (spacing_input, TestValue::Int(4)),
                    (visible_input, TestValue::Bool(false)),
                    (sensitive_input, TestValue::Bool(true)),
                ],
            )
            .expect("concrete GTK host should mount the bridge graph");

            let root = executor
                .root_widgets()
                .expect("root widget should exist")
                .into_iter()
                .next()
                .expect("window root should exist");
            let window = executor
                .host()
                .widget(&root)
                .expect("window handle should resolve")
                .downcast::<gtk::Window>()
                .expect("root should be a GTK window");
            assert_eq!(window.title().as_deref(), Some("Host"));

            let child = window.child().expect("window should have a child");
            let container = child
                .downcast::<gtk::Box>()
                .expect("window child should be a GTK box");
            assert_eq!(container.orientation(), Orientation::Vertical);
            assert_eq!(container.spacing(), 4);

            let routes = executor.event_routes();
            assert_eq!(routes.len(), 1);
            let button_handle = executor
                .widget_handle(&routes[0].instance)
                .expect("event route should point at the mounted button")
                .clone();
            let button = executor
                .host()
                .widget(&button_handle)
                .expect("button handle should resolve")
                .downcast::<gtk::Button>()
                .expect("button handle should be a GTK button");
            assert_eq!(button.label().as_deref(), Some("Save"));
            assert!(!button.is_visible());
            assert!(button.is_sensitive());

            let window_children = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked");
            let container_handle = window_children
                .first()
                .expect("window should contain the box child")
                .clone();
            let child_handles = executor
                .host()
                .child_handles(&container_handle)
                .expect("box child order should be tracked");
            assert_eq!(child_handles.len(), 2);
            let label = executor
                .host()
                .widget(&child_handles[0])
                .expect("label handle should resolve")
                .downcast::<gtk::Label>()
                .expect("first box child should be a label");
            assert_eq!(label.text().as_str(), "Runtime title");

            button.emit_clicked();
            let queued = executor.host_mut().drain_events();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].route, routes[0].id);
            assert_eq!(queued[0].value, TestValue::Unit);
        });
    }
}
