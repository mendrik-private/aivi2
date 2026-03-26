use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    rc::Rc,
    sync::Mutex,
};

use aivi_hir::{NamePath, TextLiteral, TextSegment};
use gtk::{
    Orientation,
    glib::{self, SignalHandlerId},
    prelude::*,
};

use crate::{
    GtkBoolPropertySetter, GtkChildGroupDescriptor, GtkChildMountRoute, GtkConcreteWidgetKind,
    GtkEventRoute, GtkEventRouteId, GtkEventSignal, GtkF64PropertySetter, GtkI64PropertySetter,
    GtkPropertyDescriptor, GtkPropertySetter, GtkRuntimeHost, GtkTextOrI64PropertySetter,
    GtkTextPropertySetter, GtkWidgetSchema, RuntimeSetterBinding, StaticPropertyPlan,
    StaticPropertyValue, lookup_widget_schema,
};

/// Assert that the calling code is running on the GTK main thread.
///
/// GTK4 requires all widget operations to be performed on the thread that
/// initialised GTK.  Calling any GTK widget API from another thread produces
/// undefined behaviour.  This function panics early with a clear diagnostic
/// rather than producing a silent data race or a cryptic GLib assertion.
fn assert_gtk_main_thread() {
    assert!(
        gtk::is_initialized_main_thread(),
        "GTK widget operation called from a non-main thread. \
         All GTK widget operations must be performed on the thread that initialised GTK."
    );
}

pub trait GtkHostValue: Clone + 'static {
    fn unit() -> Self;

    fn from_bool(v: bool) -> Self {
        let _ = v;
        Self::unit()
    }

    fn from_text(v: &str) -> Self {
        let _ = v;
        Self::unit()
    }

    fn as_bool(&self) -> Option<bool> {
        None
    }

    fn as_i64(&self) -> Option<i64> {
        None
    }

    fn as_f64(&self) -> Option<f64> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkQueuedWindowKeyEvent {
    pub name: Box<str>,
    pub repeated: bool,
}

/// Event queue shared between GTK signal closures and the host evaluation loop.
///
/// The host itself is `Rc<GtkConcreteHost<V>>` (single-threaded by design), so
/// `Mutex` does not introduce any cross-thread overhead.  Compared with
/// `RefCell`, `Mutex` eliminates the reentrant-borrow panic surface: if a GTK
/// callback fires while the host is draining the queue the `Mutex` will block
/// rather than panic.  Both operations are short, so the brief mutual exclusion
/// is acceptable and safe.
struct GtkEventQueue<V> {
    events: Mutex<VecDeque<GtkQueuedEvent<V>>>,
}

impl<V> Default for GtkEventQueue<V> {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
        }
    }
}

struct GtkWindowKeyQueue {
    events: Mutex<VecDeque<GtkQueuedWindowKeyEvent>>,
}

impl Default for GtkWindowKeyQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
        }
    }
}

impl GtkWindowKeyQueue {
    fn push(&self, event: GtkQueuedWindowKeyEvent) {
        self.events
            .lock()
            .expect("GtkWindowKeyQueue mutex should not be poisoned")
            .push_back(event);
    }

    fn drain(&self) -> Vec<GtkQueuedWindowKeyEvent> {
        self.events
            .lock()
            .expect("GtkWindowKeyQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }
}

impl<V> GtkEventQueue<V> {
    fn push(&self, event: GtkQueuedEvent<V>) {
        self.events
            .lock()
            .expect("GtkEventQueue mutex should not be poisoned")
            .push_back(event);
    }

    fn drain(&self) -> Vec<GtkQueuedEvent<V>> {
        self.events
            .lock()
            .expect("GtkEventQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }
}

pub struct GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    next_widget: u64,
    next_event: u64,
    widgets: BTreeMap<u64, MountedWidget>,
    events: BTreeMap<u64, MountedEvent>,
    queued_events: Rc<GtkEventQueue<V>>,
    queued_window_keys: Rc<GtkWindowKeyQueue>,
    event_notifier: Option<Rc<dyn Fn()>>,
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
            queued_events: Rc::new(GtkEventQueue::default()),
            queued_window_keys: Rc::new(GtkWindowKeyQueue::default()),
            event_notifier: None,
        }
    }
}

impl<V> GtkConcreteHost<V>
where
    V: GtkHostValue,
{
    fn assert_gtk_main_thread() {
        debug_assert!(
            gtk::is_initialized_main_thread(),
            "GtkConcreteHost methods must be called from the GTK main thread. \
             GTK is not thread-safe."
        );
    }

    pub fn set_event_notifier(&mut self, notifier: Option<Rc<dyn Fn()>>) {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.event_notifier = notifier;
    }

    pub fn widget(&self, handle: &GtkConcreteWidget) -> Option<gtk::Widget> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.widgets
            .get(&handle.0)
            .map(|mounted| mounted.widget.clone())
    }

    pub fn child_handles(&self, handle: &GtkConcreteWidget) -> Option<Vec<GtkConcreteWidget>> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.widgets.get(&handle.0).map(|mounted| {
            mounted
                .schema
                .child_groups
                .iter()
                .flat_map(|group| {
                    mounted
                        .child_groups
                        .get(group.name)
                        .into_iter()
                        .flat_map(|children| children.iter().cloned())
                })
                .collect()
        })
    }

    pub fn drain_events(&mut self) -> Vec<GtkQueuedEvent<V>> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_events.drain()
    }

    pub fn drain_window_key_events(&mut self) -> Vec<GtkQueuedWindowKeyEvent> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_window_keys.drain()
    }

    pub fn queue_window_key_event(&mut self, name: &str, repeated: bool) {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_window_keys.push(GtkQueuedWindowKeyEvent {
            name: name.into(),
            repeated,
        });
        if let Some(notifier) = &self.event_notifier {
            notifier();
        }
    }

    pub fn present_root_windows(&self) {
        assert_gtk_main_thread();
        for mounted in self.widgets.values() {
            if mounted.schema.is_window_root() && mounted.widget.parent().is_none() {
                if let Ok(window) = mounted.widget.clone().downcast::<gtk::Window>() {
                    window.present();
                } else {
                    eprintln!(
                        "aivi-gtk: present_root_windows: widget with window schema could not be \
                         downcast to gtk::Window; skipping (schema mismatch)"
                    );
                }
            }
        }
    }

    fn create_supported_widget(
        &self,
        widget: &NamePath,
    ) -> Result<(&'static GtkWidgetSchema, gtk::Widget), GtkConcreteHostError> {
        let schema = lookup_widget_schema(widget).ok_or_else(|| {
            GtkConcreteHostError::UnsupportedWidget {
                widget: widget_label(widget).to_owned().into_boxed_str(),
            }
        })?;
        let widget = match schema.kind {
            GtkConcreteWidgetKind::Window => {
                let window = gtk::Window::new();
                let key_events = self.queued_window_keys.clone();
                let notifier = self.event_notifier.clone();
                let pressed = Rc::new(Mutex::new(BTreeSet::<Box<str>>::new()));
                let released = pressed.clone();
                let controller = gtk::EventControllerKey::new();
                controller.connect_key_pressed(move |_, key, _, _| {
                    let Some(name) = normalize_window_key_name(key) else {
                        return glib::Propagation::Proceed;
                    };
                    let repeated = {
                        let mut pressed = pressed
                            .lock()
                            .expect("window key state mutex should not be poisoned");
                        !pressed.insert(name.clone())
                    };
                    key_events.push(GtkQueuedWindowKeyEvent { name, repeated });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                    glib::Propagation::Proceed
                });
                controller.connect_key_released(move |_, key, _, _| {
                    if let Some(name) = normalize_window_key_name(key) {
                        released
                            .lock()
                            .expect("window key state mutex should not be poisoned")
                            .remove(name.as_ref());
                    }
                });
                window.add_controller(controller);
                window.upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::HeaderBar => gtk::HeaderBar::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Paned => gtk::Paned::new(Orientation::Horizontal).upcast(),
            GtkConcreteWidgetKind::Box => {
                gtk::Box::new(Orientation::Vertical, 0).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::ScrolledWindow => {
                gtk::ScrolledWindow::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Frame => gtk::Frame::new(None).upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Viewport => {
                gtk::Viewport::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>)
                    .upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Label => gtk::Label::new(None).upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Button => gtk::Button::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Entry => gtk::Entry::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Switch => gtk::Switch::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::CheckButton => gtk::CheckButton::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ToggleButton => gtk::ToggleButton::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Image => gtk::Image::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Spinner => gtk::Spinner::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ProgressBar => gtk::ProgressBar::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Revealer => gtk::Revealer::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Separator => {
                gtk::Separator::new(Orientation::Horizontal).upcast::<gtk::Widget>()
            }
        };
        Ok((schema, widget))
    }

    fn mounted_snapshot(
        &self,
        handle: &GtkConcreteWidget,
    ) -> Result<(&'static GtkWidgetSchema, gtk::Widget), GtkConcreteHostError> {
        let mounted =
            self.widgets
                .get(&handle.0)
                .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                    widget: handle.clone(),
                })?;
        Ok((mounted.schema, mounted.widget.clone()))
    }

    fn group_children_snapshot(
        &self,
        handle: &GtkConcreteWidget,
        group: &'static GtkChildGroupDescriptor,
    ) -> Result<Vec<GtkConcreteWidget>, GtkConcreteHostError> {
        let mounted =
            self.widgets
                .get(&handle.0)
                .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                    widget: handle.clone(),
                })?;
        Ok(mounted
            .child_groups
            .get(group.name)
            .cloned()
            .expect("mounted widgets should track all schema child groups"))
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

    fn update_group_children(
        &mut self,
        handle: &GtkConcreteWidget,
        group: &'static GtkChildGroupDescriptor,
        children: Vec<GtkConcreteWidget>,
    ) -> Result<(), GtkConcreteHostError> {
        let mounted =
            self.widgets
                .get_mut(&handle.0)
                .ok_or_else(|| GtkConcreteHostError::UnknownWidget {
                    widget: handle.clone(),
                })?;
        mounted
            .child_groups
            .insert(group.name, children)
            .expect("mounted widgets should track all schema child groups");
        Ok(())
    }

    fn lookup_property<'a>(
        &self,
        schema: &'static GtkWidgetSchema,
        property: &'a str,
    ) -> Result<&'static GtkPropertyDescriptor, GtkConcreteHostError> {
        schema
            .property(property)
            .ok_or_else(|| GtkConcreteHostError::UnsupportedProperty {
                widget: schema.markup_name.into(),
                property: property.to_owned().into_boxed_str(),
            })
    }

    fn invalid_property_value(
        &self,
        schema: &'static GtkWidgetSchema,
        property: &GtkPropertyDescriptor,
        expected: &'static str,
    ) -> GtkConcreteHostError {
        GtkConcreteHostError::InvalidPropertyValue {
            widget: schema.markup_name.into(),
            property: property.name.into(),
            expected,
        }
    }

    fn with_blocked_widget_events<T>(
        &self,
        widget: &gtk::Widget,
        f: impl FnOnce() -> Result<T, GtkConcreteHostError>,
    ) -> Result<T, GtkConcreteHostError> {
        let signals = self
            .events
            .values()
            .filter(|mounted| mounted.widget.as_ptr() == widget.as_ptr())
            .map(|mounted| &mounted.signal)
            .collect::<Vec<_>>();
        for signal in signals.iter().copied() {
            widget.block_signal(signal);
        }
        let result = f();
        for signal in signals.iter().rev().copied() {
            widget.unblock_signal(signal);
        }
        result
    }

    fn apply_bool_property(
        &self,
        widget: &gtk::Widget,
        schema: &'static GtkWidgetSchema,
        property: &GtkPropertyDescriptor,
        value: bool,
    ) -> Result<(), GtkConcreteHostError> {
        match property.setter {
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Visible) => widget.set_visible(value),
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Sensitive) => {
                widget.set_sensitive(value)
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Hexpand) => widget.set_hexpand(value),
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Vexpand) => widget.set_vexpand(value),
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Monospace) => {
                if value {
                    widget.add_css_class("monospace");
                } else {
                    widget.remove_css_class("monospace");
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::HeaderBarShowTitleButtons) => {
                widget
                    .clone()
                    .downcast::<gtk::HeaderBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::HeaderBar",
                    })?
                    .set_show_title_buttons(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::EntryEditable) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_editable(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::SwitchActive) => {
                widget
                    .clone()
                    .downcast::<gtk::Switch>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Switch",
                    })?
                    .set_active(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::CheckButtonActive) => {
                widget
                    .clone()
                    .downcast::<gtk::CheckButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::CheckButton",
                    })?
                    .set_active(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ToggleButtonActive) => {
                widget
                    .clone()
                    .downcast::<gtk::ToggleButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ToggleButton",
                    })?
                    .set_active(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinnerSpinning) => {
                let spinner = widget.clone().downcast::<gtk::Spinner>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Spinner",
                    }
                })?;
                if value {
                    spinner.start();
                } else {
                    spinner.stop();
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::RevealerRevealed) => {
                widget
                    .clone()
                    .downcast::<gtk::Revealer>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Revealer",
                    })?
                    .set_reveal_child(value);
            }
            _ => {
                return Err(self.invalid_property_value(
                    schema,
                    property,
                    property.setter.host_value_label(),
                ));
            }
        }
        Ok(())
    }

    fn apply_text_property(
        &self,
        widget: &gtk::Widget,
        schema: &'static GtkWidgetSchema,
        property: &GtkPropertyDescriptor,
        value: &str,
    ) -> Result<(), GtkConcreteHostError> {
        match property.setter {
            GtkPropertySetter::Text(GtkTextPropertySetter::WindowTitle) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_title(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FrameLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::Frame>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Frame",
                    })?
                    .set_label(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::LabelText) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::LabelLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_label(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ButtonLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Button",
                    })?
                    .set_label(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::BoxOrientation) => {
                let orientation = parse_orientation(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "Vertical or Horizontal")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Box",
                    })?
                    .set_orientation(orientation);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PanedOrientation) => {
                let orientation = parse_orientation(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "Vertical or Horizontal")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Paned>()
                    .expect("paned widget should downcast")
                    .set_orientation(orientation);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::SeparatorOrientation) => {
                let orientation = parse_orientation(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "Vertical or Horizontal")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Separator>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Separator",
                    })?
                    .set_orientation(orientation);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntryText) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntryPlaceholderText) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_placeholder_text(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::CheckButtonLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::CheckButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::CheckButton",
                    })?
                    .set_label(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ToggleButtonLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::ToggleButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ToggleButton",
                    })?
                    .set_label(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ImageIconName) => {
                widget
                    .clone()
                    .downcast::<gtk::Image>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Image",
                    })?
                    .set_icon_name(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ImageResourcePath) => {
                widget
                    .clone()
                    .downcast::<gtk::Image>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Image",
                    })?
                    .set_resource(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ProgressBarText) => {
                widget
                    .clone()
                    .downcast::<gtk::ProgressBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ProgressBar",
                    })?
                    .set_text(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::RevealerTransitionType) => {
                use gtk::RevealerTransitionType;
                let transition = match value {
                    "None" => RevealerTransitionType::None,
                    "Crossfade" => RevealerTransitionType::Crossfade,
                    "SlideRight" => RevealerTransitionType::SlideRight,
                    "SlideLeft" => RevealerTransitionType::SlideLeft,
                    "SlideUp" => RevealerTransitionType::SlideUp,
                    "SlideDown" => RevealerTransitionType::SlideDown,
                    "SwingRight" => RevealerTransitionType::SwingRight,
                    "SwingLeft" => RevealerTransitionType::SwingLeft,
                    "SwingUp" => RevealerTransitionType::SwingUp,
                    "SwingDown" => RevealerTransitionType::SwingDown,
                    _ => {
                        return Err(self.invalid_property_value(
                            schema,
                            property,
                            "valid Revealer transition type name",
                        ));
                    }
                };
                widget
                    .clone()
                    .downcast::<gtk::Revealer>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Revealer",
                    })?
                    .set_transition_type(transition);
            }
            GtkPropertySetter::TextOrI64(GtkTextOrI64PropertySetter::BoxSpacing) => {
                let spacing = value.parse::<i32>().map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer text")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Box",
                    })?
                    .set_spacing(spacing);
            }
            _ => {
                return Err(self.invalid_property_value(
                    schema,
                    property,
                    property.setter.host_value_label(),
                ));
            }
        }
        Ok(())
    }

    fn apply_i64_property(
        &self,
        widget: &gtk::Widget,
        schema: &'static GtkWidgetSchema,
        property: &GtkPropertyDescriptor,
        value: i64,
    ) -> Result<(), GtkConcreteHostError> {
        match property.setter {
            GtkPropertySetter::TextOrI64(GtkTextOrI64PropertySetter::BoxSpacing) => {
                let spacing = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Box",
                    })?
                    .set_spacing(spacing);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::ImagePixelSize) => {
                let size = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Image>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Image",
                    })?
                    .set_pixel_size(size);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::RevealerTransitionDuration) => {
                let duration = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Revealer>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Revealer",
                    })?
                    .set_transition_duration(duration);
                Ok(())
            }
            _ => Err(self.invalid_property_value(
                schema,
                property,
                property.setter.host_value_label(),
            )),
        }
    }

    fn apply_f64_property(
        &self,
        widget: &gtk::Widget,
        schema: &'static GtkWidgetSchema,
        property: &GtkPropertyDescriptor,
        value: f64,
    ) -> Result<(), GtkConcreteHostError> {
        match property.setter {
            GtkPropertySetter::F64(GtkF64PropertySetter::ProgressBarFraction) => {
                widget
                    .clone()
                    .downcast::<gtk::ProgressBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ProgressBar",
                    })?
                    .set_fraction(value.clamp(0.0, 1.0));
                Ok(())
            }
            _ => Err(self.invalid_property_value(
                schema,
                property,
                property.setter.host_value_label(),
            )),
        }
    }

    fn repack_header_bar_children(
        &self,
        parent_widget: &gtk::Widget,
        route: GtkChildMountRoute,
        previous: &[gtk::Widget],
        next: &[gtk::Widget],
    ) {
        let header_bar = parent_widget
            .clone()
            .downcast::<gtk::HeaderBar>()
            .expect("header bar widget should downcast");
        for child in previous {
            header_bar.remove(child);
        }
        for child in next {
            match route {
                GtkChildMountRoute::HeaderBarStart => header_bar.pack_start(child),
                GtkChildMountRoute::HeaderBarEnd => header_bar.pack_end(child),
                _ => unreachable!("header bar repack requires a header bar sequence route"),
            }
        }
    }

    fn replace_sequence_children(
        &self,
        parent_widget: &gtk::Widget,
        route: GtkChildMountRoute,
        previous: &[gtk::Widget],
        next: &[gtk::Widget],
    ) {
        match route {
            GtkChildMountRoute::HeaderBarStart | GtkChildMountRoute::HeaderBarEnd => {
                self.repack_header_bar_children(parent_widget, route, previous, next);
            }
            GtkChildMountRoute::BoxChildren => {
                let box_widget = parent_widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("box widget should downcast");
                for child in previous {
                    box_widget.remove(child);
                }
                for child in next {
                    box_widget.append(child);
                }
            }
            _ => unreachable!("replace_sequence_children requires a sequence child group"),
        }
    }

    fn set_single_child(
        &self,
        parent_widget: &gtk::Widget,
        route: GtkChildMountRoute,
        child: Option<&gtk::Widget>,
    ) -> Result<(), GtkConcreteHostError> {
        match route {
            GtkChildMountRoute::WindowContent => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Window".into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::HeaderBarTitleWidget => {
                parent_widget
                    .clone()
                    .downcast::<gtk::HeaderBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "HeaderBar".into(),
                        expected_type: "gtk::HeaderBar",
                    })?
                    .set_title_widget(child);
            }
            GtkChildMountRoute::PanedStart => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Paned>()
                    .expect("paned widget should downcast")
                    .set_start_child(child);
            }
            GtkChildMountRoute::PanedEnd => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Paned>()
                    .expect("paned widget should downcast")
                    .set_end_child(child);
            }
            GtkChildMountRoute::ScrolledWindowContent => {
                parent_widget
                    .clone()
                    .downcast::<gtk::ScrolledWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ScrolledWindow".into(),
                        expected_type: "gtk::ScrolledWindow",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::FrameChild => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Frame>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Frame".into(),
                        expected_type: "gtk::Frame",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ViewportChild => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Viewport>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Viewport".into(),
                        expected_type: "gtk::Viewport",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::RevealerChild => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Revealer>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Revealer".into(),
                        expected_type: "gtk::Revealer",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::HeaderBarStart
            | GtkChildMountRoute::HeaderBarEnd
            | GtkChildMountRoute::BoxChildren => {
                unreachable!("sequence child groups are handled by explicit sequence APIs")
            }
        }
        Ok(())
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
        assert_gtk_main_thread();
        let handle = GtkConcreteWidget(self.next_widget);
        self.next_widget = self
            .next_widget
            .checked_add(1)
            .expect("concrete GTK widget handle counter should not overflow");
        let (schema, widget) = self.create_supported_widget(widget)?;
        self.widgets.insert(
            handle.0,
            MountedWidget {
                schema,
                widget,
                child_groups: schema
                    .child_groups
                    .iter()
                    .map(|group| (group.name, Vec::new()))
                    .collect(),
            },
        );
        Ok(handle)
    }

    fn apply_static_property(
        &mut self,
        widget: &Self::Widget,
        property: &StaticPropertyPlan,
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (schema, widget) = self.mounted_snapshot(widget)?;
        let descriptor = self.lookup_property(schema, property.name.text())?;
        match &property.value {
            StaticPropertyValue::ImplicitTrue => {
                self.apply_bool_property(&widget, schema, descriptor, true)
            }
            StaticPropertyValue::Text(text) if text.has_interpolation() => {
                Err(GtkConcreteHostError::InterpolatedStaticText {
                    widget: schema.markup_name.into(),
                    property: property.name.text().to_owned().into_boxed_str(),
                })
            }
            StaticPropertyValue::Text(text) => {
                self.apply_text_property(&widget, schema, descriptor, &text_literal(text))
            }
        }
    }

    fn apply_dynamic_property(
        &mut self,
        widget: &Self::Widget,
        binding: &RuntimeSetterBinding,
        value: &V,
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (schema, widget) = self.mounted_snapshot(widget)?;
        let descriptor = self.lookup_property(schema, binding.name.text())?;
        self.with_blocked_widget_events(&widget, || match descriptor.setter {
            GtkPropertySetter::Bool(_) => {
                let value = value.as_bool().ok_or_else(|| {
                    self.invalid_property_value(
                        schema,
                        descriptor,
                        descriptor.setter.host_value_label(),
                    )
                })?;
                self.apply_bool_property(&widget, schema, descriptor, value)
            }
            GtkPropertySetter::Text(_) => {
                let value = value.as_text().ok_or_else(|| {
                    self.invalid_property_value(
                        schema,
                        descriptor,
                        descriptor.setter.host_value_label(),
                    )
                })?;
                self.apply_text_property(&widget, schema, descriptor, value)
            }
            GtkPropertySetter::TextOrI64(_) => {
                if let Some(value) = value.as_i64() {
                    self.apply_i64_property(&widget, schema, descriptor, value)
                } else if let Some(value) = value.as_text() {
                    self.apply_text_property(&widget, schema, descriptor, value)
                } else {
                    Err(self.invalid_property_value(
                        schema,
                        descriptor,
                        descriptor.setter.host_value_label(),
                    ))
                }
            }
            GtkPropertySetter::I64(_) => {
                let value = value.as_i64().ok_or_else(|| {
                    self.invalid_property_value(
                        schema,
                        descriptor,
                        descriptor.setter.host_value_label(),
                    )
                })?;
                self.apply_i64_property(&widget, schema, descriptor, value)
            }
            GtkPropertySetter::F64(_) => {
                let value = value.as_f64().ok_or_else(|| {
                    self.invalid_property_value(
                        schema,
                        descriptor,
                        descriptor.setter.host_value_label(),
                    )
                })?;
                self.apply_f64_property(&widget, schema, descriptor, value)
            }
        })
    }

    fn connect_event(
        &mut self,
        widget: &Self::Widget,
        route: &GtkEventRoute,
    ) -> Result<Self::EventHandle, Self::Error> {
        assert_gtk_main_thread();
        let (schema, widget) = self.mounted_snapshot(widget)?;
        let handle = GtkConcreteEventHandle(self.next_event);
        self.next_event = self
            .next_event
            .checked_add(1)
            .expect("concrete GTK event handle counter should not overflow");
        let queue = self.queued_events.clone();
        let notifier = self.event_notifier.clone();
        let route_id = route.id;
        let event = schema.event(route.binding.name.text()).ok_or_else(|| {
            GtkConcreteHostError::UnsupportedEvent {
                widget: schema.markup_name.into(),
                event: route.binding.name.text().to_owned().into_boxed_str(),
            }
        })?;
        // NOTE: Signal handler IDs are stored in `MountedEvent` but are only
        // disconnected when `disconnect_event` is explicitly called. If a widget
        // is destroyed without going through `disconnect_event`, the handler ID
        // becomes stale and can cause use-after-free or ghost callbacks if the
        // handler closure captures live state (e.g. an Rc to the event queue).
        //
        // TODO: Track handler IDs alongside widget lifetimes and disconnect them
        // automatically in `release_widget`, before the GTK object is dropped.
        // Consider hooking into the widget's `destroy` signal as a safety net.
        let signal = match event.signal {
            GtkEventSignal::ButtonClicked => widget
                .clone()
                .downcast::<gtk::Button>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Button",
                })?
                .connect_clicked(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
            GtkEventSignal::EntryChanged => widget
                .clone()
                .downcast::<gtk::Entry>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Entry",
                })?
                .connect_changed(move |entry| {
                    let text = entry.text();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
            GtkEventSignal::EntryActivated => widget
                .clone()
                .downcast::<gtk::Entry>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Entry",
                })?
                .connect_activate(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
            GtkEventSignal::SwitchToggled => widget
                .clone()
                .downcast::<gtk::Switch>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Switch",
                })?
                .connect_active_notify(move |switch| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(switch.is_active()),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
            GtkEventSignal::CheckButtonToggled => widget
                .clone()
                .downcast::<gtk::CheckButton>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::CheckButton",
                })?
                .connect_toggled(move |btn| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(btn.is_active()),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
            GtkEventSignal::ToggleButtonToggled => widget
                .clone()
                .downcast::<gtk::ToggleButton>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::ToggleButton",
                })?
                .connect_toggled(move |btn| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(btn.is_active()),
                    });
                    if let Some(notifier) = &notifier {
                        notifier();
                    }
                }),
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
        assert_gtk_main_thread();
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
        group: &'static GtkChildGroupDescriptor,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (schema, parent_widget) = self.mounted_snapshot(parent)?;
        let current_children = self.group_children_snapshot(parent, group)?;
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
        match group.container {
            crate::GtkChildContainerKind::Single => {
                if current_children.len() + children.len() > 1 || index != 0 {
                    return Err(GtkConcreteHostError::UnsupportedParentOperation {
                        parent: parent.clone(),
                        widget: schema.markup_name.into(),
                        operation: "insert_children".into(),
                    });
                }
                let child = child_widgets.first().ok_or_else(|| {
                    GtkConcreteHostError::UnsupportedParentOperation {
                        parent: parent.clone(),
                        widget: schema.markup_name.into(),
                        operation: "insert_children".into(),
                    }
                })?;
                self.set_single_child(&parent_widget, group.mount, Some(child))?;
                next_children.splice(index..index, children.iter().cloned());
            }
            crate::GtkChildContainerKind::Sequence => {
                next_children.splice(index..index, children.iter().cloned());
                let next_widgets = next_children
                    .iter()
                    .map(|child| self.widget_object(child))
                    .collect::<Result<Vec<_>, _>>()?;
                self.replace_sequence_children(
                    &parent_widget,
                    group.mount,
                    &current_children
                        .iter()
                        .map(|child| self.widget_object(child))
                        .collect::<Result<Vec<_>, _>>()?,
                    &next_widgets,
                );
            }
        }
        self.update_group_children(parent, group, next_children)
    }

    fn remove_children(
        &mut self,
        parent: &Self::Widget,
        group: &'static GtkChildGroupDescriptor,
        index: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (_, parent_widget) = self.mounted_snapshot(parent)?;
        let current_children = self.group_children_snapshot(parent, group)?;
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
        let mut next_children = current_children.clone();
        match group.container {
            crate::GtkChildContainerKind::Single => {
                self.set_single_child(&parent_widget, group.mount, None)?;
                next_children.clear();
            }
            crate::GtkChildContainerKind::Sequence => {
                let previous_widgets = current_children
                    .iter()
                    .map(|child| self.widget_object(child))
                    .collect::<Result<Vec<_>, _>>()?;
                next_children.drain(index..index + children.len());
                let next_widgets = next_children
                    .iter()
                    .map(|child| self.widget_object(child))
                    .collect::<Result<Vec<_>, _>>()?;
                self.replace_sequence_children(
                    &parent_widget,
                    group.mount,
                    &previous_widgets,
                    &next_widgets,
                );
            }
        }
        self.update_group_children(parent, group, next_children)
    }

    fn move_children(
        &mut self,
        parent: &Self::Widget,
        group: &'static GtkChildGroupDescriptor,
        from: usize,
        count: usize,
        to: usize,
        children: &[Self::Widget],
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (schema, parent_widget) = self.mounted_snapshot(parent)?;
        let current_children = self.group_children_snapshot(parent, group)?;
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
        match group.container {
            crate::GtkChildContainerKind::Sequence => {
                let mut next_children = current_children.clone();
                let moved: Vec<_> = next_children.drain(from..from + count).collect();
                next_children.splice(to..to, moved.iter().cloned());
                let previous_widgets = current_children
                    .iter()
                    .map(|child| self.widget_object(child))
                    .collect::<Result<Vec<_>, _>>()?;
                let next_widgets = next_children
                    .iter()
                    .map(|child| self.widget_object(child))
                    .collect::<Result<Vec<_>, _>>()?;
                self.replace_sequence_children(
                    &parent_widget,
                    group.mount,
                    &previous_widgets,
                    &next_widgets,
                );
                self.update_group_children(parent, group, next_children)
            }
            crate::GtkChildContainerKind::Single if from == 0 && count == 1 && to == 0 => Ok(()),
            crate::GtkChildContainerKind::Single => {
                Err(GtkConcreteHostError::UnsupportedParentOperation {
                    parent: parent.clone(),
                    widget: schema.markup_name.into(),
                    operation: "move_children".into(),
                })
            }
        }
    }

    fn set_widget_visibility(
        &mut self,
        widget: &Self::Widget,
        visible: bool,
    ) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
        let (_, widget) = self.mounted_snapshot(widget)?;
        widget.set_visible(visible);
        Ok(())
    }

    fn release_widget(&mut self, widget: Self::Widget) -> Result<(), Self::Error> {
        assert_gtk_main_thread();
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
        if mounted.schema.is_window_root() {
            match mounted.widget.downcast::<gtk::Window>() {
                Ok(window) => window.close(),
                Err(_) => {
                    eprintln!(
                        "aivi-gtk: release_widget: widget with window schema could not be \
                         downcast to gtk::Window; window will not be closed (schema mismatch)"
                    );
                }
            }
        }
        Ok(())
    }
}

struct MountedWidget {
    schema: &'static GtkWidgetSchema,
    widget: gtk::Widget,
    child_groups: BTreeMap<&'static str, Vec<GtkConcreteWidget>>,
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
    InterpolatedStaticText {
        widget: Box<str>,
        property: Box<str>,
    },
    /// A GTK widget could not be downcast to the expected concrete type.
    ///
    /// This indicates a schema-to-widget-kind mismatch: the widget was created
    /// with a different concrete type than the property/event setter expected.
    /// Rather than panicking, the operation is aborted and the error is
    /// propagated so the caller can log or recover gracefully.
    WidgetDowncastFailed {
        widget: Box<str>,
        expected_type: &'static str,
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
            Self::InterpolatedStaticText { widget, property } => write!(
                f,
                "GTK host cannot mount interpolated static text for property `{property}` on widget `{widget}`"
            ),
            Self::WidgetDowncastFailed {
                widget,
                expected_type,
            } => write!(
                f,
                "GTK host widget `{widget}` could not be downcast to the expected type `{expected_type}`; \
                 this indicates a schema-to-widget-kind mismatch"
            ),
        }
    }
}

impl Error for GtkConcreteHostError {}

/// Return the catalog label for a widget path.
///
/// **Invariant**: `NamePath` is constructed by the HIR parser which guarantees
/// at least one segment per path node.  An empty path is a parser bug, not a
/// user error, so the `expect` here is a programmer assertion rather than a
/// recoverable condition (I4).
fn widget_label(path: &NamePath) -> &str {
    lookup_widget_schema(path)
        .map(|schema| schema.markup_name)
        .unwrap_or_else(|| {
            path.segments()
                .iter()
                .last()
                .expect("NamePath must contain at least one segment — this is a parser invariant")
                .text()
        })
}

fn normalize_window_key_name(key: gtk::gdk::Key) -> Option<Box<str>> {
    let name = key.name()?;
    let mapped = match name.as_str() {
        "Up" => "ArrowUp".to_owned(),
        "Down" => "ArrowDown".to_owned(),
        "Left" => "ArrowLeft".to_owned(),
        "Right" => "ArrowRight".to_owned(),
        "space" => "Space".to_owned(),
        "Return" | "KP_Enter" => "Enter".to_owned(),
        other => other.to_owned(),
    };
    Some(mapped.into_boxed_str())
}

fn text_literal(text: &TextLiteral) -> String {
    text.segments
        .iter()
        .map(|segment| match segment {
            TextSegment::Text(fragment) => fragment.raw.as_ref(),
            TextSegment::Interpolation(_) => {
                unreachable!("interpolated static text should be rejected before rendering")
            }
        })
        .collect()
}

fn parse_orientation(value: &str) -> Option<Orientation> {
    match value.trim() {
        "Vertical" | "vertical" => Some(Orientation::Vertical),
        "Horizontal" | "horizontal" => Some(Orientation::Horizontal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::{FileId, SourceDatabase, SourceSpan, Span};
    use aivi_hir::{
        ExprId, Item, MarkupNodeId, Name, TextFragment, TextInterpolation, TextLiteral,
        TextSegment, lower_module,
    };
    use aivi_runtime::InputHandle;
    use aivi_syntax::parse_module;
    use gtk::prelude::*;

    use crate::{
        AttributeSite, GtkBridgeGraph, GtkBridgeNodeKind, GtkRuntimeExecutor,
        RuntimePropertyBinding, StableNodeId, StaticPropertyPlan, StaticPropertyValue,
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

        fn from_bool(v: bool) -> Self {
            Self::Bool(v)
        }

        fn from_text(v: &str) -> Self {
            Self::Text(v.to_owned())
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
                    if super::widget_label(&widget.widget) == widget_name =>
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

    fn span(start: usize, end: usize) -> SourceSpan {
        SourceSpan::new(FileId::new(0), Span::from(start..end))
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

    #[test]
    fn concrete_host_mounts_expanded_widget_catalog_and_captures_entry_activation() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-expanded.aivi",
                r#"
val query = "Runtime query"
val canEdit = False
val isEnabled = True
val submit = True
val view =
    <Window title="Host">
        <ScrolledWindow>
            <Box>
                <Entry text={query} placeholderText="Search" editable={canEdit} onActivate={submit} />
                <Switch active={isEnabled} />
            </Box>
        </ScrolledWindow>
    </Window>
"#,
            );
            let text_input = find_widget_input(&graph, "Entry", "text");
            let editable_input = find_widget_input(&graph, "Entry", "editable");
            let active_input = find_widget_input(&graph, "Switch", "active");
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (text_input, TestValue::Text("Runtime query".to_string())),
                    (editable_input, TestValue::Bool(false)),
                    (active_input, TestValue::Bool(true)),
                ],
            )
            .expect("concrete GTK host should mount the expanded widget slice");

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
            let scrolled = window
                .child()
                .expect("window should host the scrolled window child")
                .downcast::<gtk::ScrolledWindow>()
                .expect("window child should be a scrolled window");
            assert!(
                scrolled.child().is_some(),
                "scrolled window should host the box child (possibly through a viewport wrapper)"
            );

            let window_children = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked");
            let scrolled_handle = window_children
                .first()
                .expect("window should contain the scrolled window child")
                .clone();
            let scrolled_children = executor
                .host()
                .child_handles(&scrolled_handle)
                .expect("scrolled window child order should be tracked");
            assert_eq!(scrolled_children.len(), 1);

            let child_handles = executor
                .host()
                .child_handles(
                    scrolled_children
                        .first()
                        .expect("scrolled window should contain the box child"),
                )
                .expect("box child order should be tracked");
            assert_eq!(child_handles.len(), 2);

            let entry = executor
                .host()
                .widget(&child_handles[0])
                .expect("entry handle should resolve")
                .downcast::<gtk::Entry>()
                .expect("first box child should be an entry");
            assert_eq!(entry.text().as_str(), "Runtime query");
            assert_eq!(
                entry.property::<Option<String>>("placeholder-text"),
                Some("Search".to_string())
            );
            assert!(!entry.property::<bool>("editable"));

            let switch = executor
                .host()
                .widget(&child_handles[1])
                .expect("switch handle should resolve")
                .downcast::<gtk::Switch>()
                .expect("second box child should be a switch");
            assert!(switch.property::<bool>("active"));

            let routes = executor.event_routes();
            assert_eq!(routes.len(), 1);
            let entry_handle = executor
                .widget_handle(&routes[0].instance)
                .expect("event route should point at the mounted entry")
                .clone();
            assert_eq!(entry_handle, child_handles[0]);

            entry.emit_by_name::<()>("activate", &[]);
            let queued = executor.host_mut().drain_events();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].route, routes[0].id);
            assert_eq!(queued[0].value, TestValue::Unit);
        });
    }

    #[test]
    fn concrete_host_blocks_programmatic_entry_updates_and_captures_entry_changes() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-entry-change.aivi",
                r#"
val query = "Runtime query"
sig changed : Signal Text

val view =
    <Window title="Host">
        <Entry text={query} onChange={changed} />
    </Window>
"#,
            );
            let text_input = find_widget_input(&graph, "Entry", "text");
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [(text_input, TestValue::Text("Runtime query".to_string()))],
            )
            .expect("concrete GTK host should mount an entry change handler");

            let routes = executor.event_routes();
            assert_eq!(routes.len(), 1);
            let entry_handle = executor
                .widget_handle(&routes[0].instance)
                .expect("event route should point at the mounted entry")
                .clone();
            let entry = executor
                .host()
                .widget(&entry_handle)
                .expect("entry handle should resolve")
                .downcast::<gtk::Entry>()
                .expect("root child should be an entry");
            assert_eq!(entry.text().as_str(), "Runtime query");

            executor
                .set_property_for_instance(
                    &routes[0].instance,
                    text_input,
                    TestValue::Text("Server query".to_string()),
                )
                .expect("programmatic entry updates should still apply");
            assert_eq!(entry.text().as_str(), "Server query");
            assert!(
                executor.host_mut().drain_events().is_empty(),
                "programmatic entry text updates should not re-emit onChange"
            );

            entry.set_text("Typed query");
            let queued = executor.host_mut().drain_events();
            assert!(
                !queued.is_empty(),
                "entry changes should publish at least one onChange event"
            );
            assert!(queued.iter().all(|event| event.route == routes[0].id));
            assert_eq!(
                queued
                    .last()
                    .expect("entry changes should queue one latest event")
                    .value,
                TestValue::Text("Typed query".to_string())
            );
        });
    }

    #[test]
    fn concrete_host_mounts_additional_common_widgets_and_captures_switch_toggles() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-additional-widgets.aivi",
                r#"
val showButtons = False
val isEnabled = True
val toggled = True
val view =
    <Window title="Host">
        <Viewport>
            <Frame label="Controls">
                <Box>
                    <HeaderBar showTitleButtons={showButtons}>
                        <Label text="Profile" />
                    </HeaderBar>
                    <Separator orientation="Horizontal" />
                    <Switch active={isEnabled} onToggle={toggled} />
                </Box>
            </Frame>
        </Viewport>
    </Window>
"#,
            );
            let show_title_buttons_input =
                find_widget_input(&graph, "HeaderBar", "showTitleButtons");
            let active_input = find_widget_input(&graph, "Switch", "active");
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (show_title_buttons_input, TestValue::Bool(false)),
                    (active_input, TestValue::Bool(true)),
                ],
            )
            .expect("concrete GTK host should mount the additional widget slice");

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
            let viewport = window
                .child()
                .expect("window should host the viewport child")
                .downcast::<gtk::Viewport>()
                .expect("window child should be a viewport");
            let frame = viewport
                .child()
                .expect("viewport should host the frame child")
                .downcast::<gtk::Frame>()
                .expect("viewport child should be a frame");
            assert_eq!(frame.label().as_deref(), Some("Controls"));

            let window_children = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked");
            let viewport_handle = window_children
                .first()
                .expect("window should contain the viewport child")
                .clone();
            let frame_handle = executor
                .host()
                .child_handles(&viewport_handle)
                .expect("viewport child order should be tracked")
                .first()
                .expect("viewport should contain the frame child")
                .clone();
            let box_handle = executor
                .host()
                .child_handles(&frame_handle)
                .expect("frame child order should be tracked")
                .first()
                .expect("frame should contain the box child")
                .clone();
            let child_handles = executor
                .host()
                .child_handles(&box_handle)
                .expect("box child order should be tracked");
            assert_eq!(child_handles.len(), 3);

            let header_bar = executor
                .host()
                .widget(&child_handles[0])
                .expect("header bar handle should resolve")
                .downcast::<gtk::HeaderBar>()
                .expect("first box child should be a header bar");
            assert!(!header_bar.property::<bool>("show-title-buttons"));
            let title_widget = header_bar
                .title_widget()
                .expect("header bar should keep the title widget child")
                .downcast::<gtk::Label>()
                .expect("header bar title widget should be a label");
            assert_eq!(title_widget.text().as_str(), "Profile");

            let separator = executor
                .host()
                .widget(&child_handles[1])
                .expect("separator handle should resolve")
                .downcast::<gtk::Separator>()
                .expect("second box child should be a separator");
            assert_eq!(separator.orientation(), Orientation::Horizontal);

            let switch = executor
                .host()
                .widget(&child_handles[2])
                .expect("switch handle should resolve")
                .downcast::<gtk::Switch>()
                .expect("third box child should be a switch");
            assert!(switch.is_active());

            let routes = executor.event_routes();
            assert_eq!(routes.len(), 1);
            let switch_handle = executor
                .widget_handle(&routes[0].instance)
                .expect("event route should point at the mounted switch")
                .clone();
            assert_eq!(switch_handle, child_handles[2]);

            executor
                .set_property_for_instance(
                    &routes[0].instance,
                    active_input,
                    TestValue::Bool(false),
                )
                .expect("programmatic switch updates should still apply");
            assert!(!switch.is_active());
            assert!(
                executor.host_mut().drain_events().is_empty(),
                "programmatic switch updates should not re-emit onToggle"
            );

            switch.set_active(true);
            let queued = executor.host_mut().drain_events();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].route, routes[0].id);
            assert_eq!(queued[0].value, TestValue::Bool(true));
        });
    }

    #[test]
    fn concrete_host_applies_label_monospace_class() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-label-monospace.aivi",
                r#"
val view =
    <Window title="Host">
        <Label text="Board" monospace />
    </Window>
"#,
            );
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [],
            )
            .expect("concrete GTK host should mount a monospace label");

            let root = executor
                .root_widgets()
                .expect("root widget should exist")
                .into_iter()
                .next()
                .expect("window root should exist");
            let label_handle = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked")
                .first()
                .expect("window should contain the label child")
                .clone();
            let label = executor
                .host()
                .widget(&label_handle)
                .expect("label handle should resolve")
                .downcast::<gtk::Label>()
                .expect("window child should be a label");

            assert!(label.has_css_class("monospace"));
        });
    }

    #[test]
    fn concrete_host_mounts_named_child_groups_for_paned_and_header_bar() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-named-groups.aivi",
                r#"
val showButtons = False
val view =
    <Window title="Host">
        <Paned orientation="Horizontal">
            <Paned.start>
                <Label text="Primary" />
            </Paned.start>
            <Paned.end>
                <HeaderBar showTitleButtons={showButtons}>
                    <HeaderBar.start>
                        <Button label="Back" />
                    </HeaderBar.start>
                    <HeaderBar.titleWidget>
                        <Label text="Inbox" />
                    </HeaderBar.titleWidget>
                    <HeaderBar.end>
                        <Button label="More" />
                    </HeaderBar.end>
                </HeaderBar>
            </Paned.end>
        </Paned>
    </Window>
"#,
            );
            let show_title_buttons_input =
                find_widget_input(&graph, "HeaderBar", "showTitleButtons");
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [(show_title_buttons_input, TestValue::Bool(false))],
            )
            .expect("concrete GTK host should mount named child groups");

            let root = executor
                .root_widgets()
                .expect("root widget should exist")
                .into_iter()
                .next()
                .expect("window root should exist");
            let window_children = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked");
            let paned_handle = window_children
                .first()
                .expect("window should contain the paned child")
                .clone();

            let paned = executor
                .host()
                .widget(&paned_handle)
                .expect("paned handle should resolve")
                .downcast::<gtk::Paned>()
                .expect("window child should be a paned widget");
            let paned_children = executor
                .host()
                .child_handles(&paned_handle)
                .expect("paned child handles should be tracked");
            let start_child = paned
                .start_child()
                .expect("paned start child should be mounted")
                .downcast::<gtk::Label>()
                .expect("paned start child should be a label");
            assert_eq!(start_child.text().as_str(), "Primary");

            let end_child = paned
                .end_child()
                .expect("paned end child should be mounted")
                .downcast::<gtk::HeaderBar>()
                .expect("paned end child should be a header bar");
            assert!(!end_child.property::<bool>("show-title-buttons"));
            let title_widget = end_child
                .title_widget()
                .expect("header bar title widget should be mounted")
                .downcast::<gtk::Label>()
                .expect("header bar title widget should be a label");
            assert_eq!(title_widget.text().as_str(), "Inbox");

            let header_children = executor
                .host()
                .child_handles(&paned_children[1])
                .expect("header bar child order should be tracked");
            assert_eq!(header_children.len(), 3);

            let back_button = executor
                .host()
                .widget(&header_children[0])
                .expect("header bar start child should resolve")
                .downcast::<gtk::Button>()
                .expect("header bar start child should be a button");
            assert_eq!(back_button.label().as_deref(), Some("Back"));

            let more_button = executor
                .host()
                .widget(&header_children[1])
                .expect("header bar end child should resolve")
                .downcast::<gtk::Button>()
                .expect("header bar end child should be a button");
            assert_eq!(more_button.label().as_deref(), Some("More"));
        });
    }

    #[test]
    fn concrete_host_attaches_window_key_controllers() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-window-keys.aivi",
                r#"
val view =
    <Window title="Host" />
"#,
            );
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [],
            )
            .expect("concrete GTK host should mount a static window");
            let root = executor
                .root_widgets()
                .expect("window root should exist")
                .into_iter()
                .next()
                .expect("expected one window root");
            let window = executor
                .host()
                .widget(&root)
                .expect("window handle should resolve")
                .downcast::<gtk::Window>()
                .expect("root should be a GTK window");
            assert!(
                window.observe_controllers().n_items() > 0,
                "window widgets should install a key controller for @source window.keyDown events"
            );
        });
    }

    #[test]
    fn concrete_host_rejects_interpolated_static_text() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-static-property-guard.aivi",
                r#"
val view =
    <Window title="Host" />
"#,
            );
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                std::iter::empty::<(InputHandle, TestValue)>(),
            )
            .expect("concrete GTK host should mount a static window");
            let widget = executor
                .root_widgets()
                .expect("root widget should exist")
                .into_iter()
                .next()
                .expect("window root should exist");
            let plan = StaticPropertyPlan {
                site: AttributeSite {
                    owner: StableNodeId::Markup(MarkupNodeId::from_raw(0)),
                    index: 0,
                    span: span(0, 18),
                },
                name: Name::new("title", span(0, 5)).unwrap(),
                value: StaticPropertyValue::Text(TextLiteral {
                    segments: vec![
                        TextSegment::Text(TextFragment {
                            raw: "Hello ".into(),
                            span: span(6, 12),
                        }),
                        TextSegment::Interpolation(TextInterpolation {
                            span: span(12, 18),
                            expr: ExprId::from_raw(0),
                        }),
                    ],
                }),
            };
            let error = executor
                .host_mut()
                .apply_static_property(&widget, &plan)
                .expect_err("static GTK text interpolation should be rejected explicitly");
            assert!(matches!(
                error,
                GtkConcreteHostError::InterpolatedStaticText { property, .. }
                    if property.as_ref() == "title"
            ));
        });
    }

    #[test]
    fn concrete_host_moves_only_the_requested_child_range() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-move-children.aivi",
                r#"
val view =
    <Window title="Host">
        <Box>
            <Label text="A" />
            <Label text="B" />
            <Label text="C" />
        </Box>
    </Window>
"#,
            );
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                std::iter::empty::<(InputHandle, TestValue)>(),
            )
            .expect("concrete GTK host should mount the bridge graph");

            let root = executor
                .root_widgets()
                .expect("root widget should exist")
                .into_iter()
                .next()
                .expect("window root should exist");
            let container_handle = executor
                .host()
                .child_handles(&root)
                .expect("window child order should be tracked")
                .into_iter()
                .next()
                .expect("window should contain the box child");
            let before = executor
                .host()
                .child_handles(&container_handle)
                .expect("box child order should be tracked");

            executor
                .host_mut()
                .move_children(
                    &container_handle,
                    crate::lookup_widget_schema_by_name("Box")
                        .and_then(|schema| schema.child_group("children"))
                        .expect("Box should expose its default children group"),
                    0,
                    1,
                    2,
                    &[before[0].clone()],
                )
                .expect("moving a single mounted child should succeed");

            let after = executor
                .host()
                .child_handles(&container_handle)
                .expect("box child order should be tracked after the move");
            let labels = after
                .iter()
                .map(|handle| {
                    executor
                        .host()
                        .widget(handle)
                        .expect("label handle should resolve")
                        .downcast::<gtk::Label>()
                        .expect("moved children should stay labels")
                        .text()
                        .to_string()
                })
                .collect::<Vec<_>>();
            assert_eq!(labels, vec!["B", "C", "A"]);
        });
    }
}
