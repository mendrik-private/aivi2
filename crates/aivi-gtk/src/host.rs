use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    rc::Rc,
    sync::Mutex,
};

use adw::prelude::*;
use aivi_hir::{NamePath, TextLiteral, TextSegment};
use gtk::{
    Orientation,
    glib::{self, SignalHandlerId},
    prelude::*,
};
use libadwaita as adw;
use webkit6::prelude::*;
use webkit6::{NetworkSession, PolicyDecisionType, WebView};

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

const AIVI_WIDGET_STYLE_CSS: &str = r#"
button.aivi-compact-button {
    padding: 2px;
    min-width: 0;
    min-height: 0;
}

button.aivi-animate-opacity {
    transition: opacity 80ms ease-in-out;
}
"#;

const AIVI_WEB_VIEW_DEFAULT_CSP: &str = concat!(
    "default-src 'none'; ",
    "img-src data: blob: file: resource: cid: aivi:; ",
    "media-src data: blob: file: resource: cid: aivi:; ",
    "style-src 'unsafe-inline' data: blob: file: resource:; ",
    "font-src data: blob: file: resource:; ",
    "connect-src 'none'; ",
    "script-src 'none'; ",
    "object-src 'none'; ",
    "frame-src 'none'; ",
    "worker-src 'none'; ",
    "form-action 'none'; ",
    "navigate-to 'none';"
);

thread_local! {
    static AIVI_WIDGET_STYLE_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

fn ensure_aivi_widget_styles() {
    assert_gtk_main_thread();
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    AIVI_WIDGET_STYLE_PROVIDER.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_data(AIVI_WIDGET_STYLE_CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        *slot.borrow_mut() = Some(provider);
    });
}

fn configure_aivi_web_view(web_view: &WebView) {
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(web_view) {
        settings.set_enable_javascript(false);
        settings.set_enable_javascript_markup(false);
        settings.set_enable_html5_local_storage(false);
        settings.set_enable_hyperlink_auditing(false);
        settings.set_enable_dns_prefetching(false);
        settings.set_enable_media(false);
        settings.set_enable_media_capabilities(false);
        settings.set_enable_media_stream(false);
        settings.set_enable_mediasource(false);
        settings.set_enable_page_cache(false);
        settings.set_enable_webgl(false);
        settings.set_allow_file_access_from_file_urls(false);
        settings.set_allow_universal_access_from_file_urls(false);
        settings.set_auto_load_images(true);
    }
    web_view.connect_permission_request(|_, request| {
        request.deny();
        true
    });
    web_view.connect_decide_policy(|_, decision, decision_type| match decision_type {
        PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction => {
            decision.ignore();
            true
        }
        PolicyDecisionType::Response | PolicyDecisionType::__Unknown(_) => false,
        _ => false,
    });
}

fn create_collection_item_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let list_item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory setup should receive gtk::ListItem");
        let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_item.set_child(Some(&wrapper));
        list_item.set_activatable(true);
    });
    factory.connect_bind(|_, item| {
        let list_item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory bind should receive gtk::ListItem");
        let wrapper = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
            .expect("collection list items should install a box wrapper during setup");
        while let Some(child) = wrapper.first_child() {
            wrapper.remove(&child);
        }
        let Some(item_object) = list_item.item() else {
            return;
        };
        let boxed = item_object
            .downcast::<glib::BoxedAnyObject>()
            .expect("collection list items should store glib::BoxedAnyObject");
        let child = boxed.borrow::<gtk::Widget>().clone();
        if let Some(parent) = child
            .parent()
            .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        {
            parent.remove(&child);
        }
        wrapper.append(&child);
    });
    factory.connect_unbind(|_, item| {
        let list_item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory unbind should receive gtk::ListItem");
        let wrapper = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
            .expect("collection list items should keep a box wrapper during unbind");
        while let Some(child) = wrapper.first_child() {
            wrapper.remove(&child);
        }
    });
    factory
}

fn selection_model_store(model: Option<gtk::SelectionModel>) -> Option<gtk::gio::ListStore> {
    let model = model?;
    if let Ok(no_selection) = model.clone().downcast::<gtk::NoSelection>() {
        return no_selection
            .model()
            .and_then(|model| model.downcast::<gtk::gio::ListStore>().ok());
    }
    if let Ok(single_selection) = model.downcast::<gtk::SingleSelection>() {
        return single_selection
            .model()
            .and_then(|model| model.downcast::<gtk::gio::ListStore>().ok());
    }
    None
}

fn replace_collection_store_children(store: &gtk::gio::ListStore, next: &[gtk::Widget]) {
    let boxed = next
        .iter()
        .cloned()
        .map(glib::BoxedAnyObject::new)
        .collect::<Vec<_>>();
    store.splice(0, store.n_items(), &boxed);
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

    fn from_f64(v: f64) -> Self {
        let _ = v;
        Self::unit()
    }

    fn from_i64(v: i64) -> Self {
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

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkWindowKeyQueue mutex should not be poisoned")
            .is_empty()
    }
}

/// Coalescing queue for `gtk.darkMode` events.  Only the latest value matters;
/// rapid theme changes collapse to the final state seen before the next drain.
struct GtkDarkModeQueue {
    events: Mutex<VecDeque<bool>>,
}

impl Default for GtkDarkModeQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
        }
    }
}

impl GtkDarkModeQueue {
    fn push(&self, is_dark: bool) {
        let mut q = self
            .events
            .lock()
            .expect("GtkDarkModeQueue mutex should not be poisoned");
        q.clear(); // Only the latest value matters.
        q.push_back(is_dark);
    }

    fn drain(&self) -> Vec<bool> {
        self.events
            .lock()
            .expect("GtkDarkModeQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkDarkModeQueue mutex should not be poisoned")
            .is_empty()
    }
}

/// Coalescing queue for `clipboard.changed` events. Only the latest clipboard text matters;
/// rapid clipboard changes collapse to the final text seen before the next drain.
struct GtkClipboardQueue {
    events: Mutex<VecDeque<String>>,
}

impl Default for GtkClipboardQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
        }
    }
}

impl GtkClipboardQueue {
    fn push(&self, text: String) {
        let mut q = self
            .events
            .lock()
            .expect("GtkClipboardQueue mutex should not be poisoned");
        q.clear(); // Only the latest value matters.
        q.push_back(text);
    }

    fn drain(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("GtkClipboardQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkClipboardQueue mutex should not be poisoned")
            .is_empty()
    }
}

struct GtkWindowSizeQueue {
    events: Mutex<VecDeque<(i32, i32)>>,
    last_seen: Mutex<Option<(i32, i32)>>,
}

impl Default for GtkWindowSizeQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            last_seen: Mutex::new(None),
        }
    }
}

impl GtkWindowSizeQueue {
    fn push(&self, event: (i32, i32)) -> bool {
        let mut last_seen = self
            .last_seen
            .lock()
            .expect("GtkWindowSizeQueue last_seen mutex should not be poisoned");
        if last_seen.is_some_and(|previous| previous == event) {
            return false;
        }
        let mut guard = self
            .events
            .lock()
            .expect("GtkWindowSizeQueue mutex should not be poisoned");
        guard.clear();
        guard.push_back(event);
        *last_seen = Some(event);
        true
    }
    fn drain(&self) -> Vec<(i32, i32)> {
        self.events
            .lock()
            .expect("GtkWindowSizeQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkWindowSizeQueue mutex should not be poisoned")
            .is_empty()
    }
}

struct GtkWindowFocusQueue {
    events: Mutex<VecDeque<bool>>,
    last_seen: Mutex<Option<bool>>,
}

impl Default for GtkWindowFocusQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            last_seen: Mutex::new(None),
        }
    }
}

impl GtkWindowFocusQueue {
    fn push(&self, event: bool) -> bool {
        let mut last_seen = self
            .last_seen
            .lock()
            .expect("GtkWindowFocusQueue last_seen mutex should not be poisoned");
        if last_seen.is_some_and(|previous| previous == event) {
            return false;
        }
        let mut guard = self
            .events
            .lock()
            .expect("GtkWindowFocusQueue mutex should not be poisoned");
        guard.clear();
        guard.push_back(event);
        *last_seen = Some(event);
        true
    }
    fn drain(&self) -> Vec<bool> {
        self.events
            .lock()
            .expect("GtkWindowFocusQueue mutex should not be poisoned")
            .drain(..)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkWindowFocusQueue mutex should not be poisoned")
            .is_empty()
    }
}

fn notify_pending_event(notifier: &Rc<RefCell<Option<Rc<dyn Fn()>>>>) {
    if let Some(notifier) = notifier.borrow().clone() {
        notifier();
    }
}

fn queue_window_size_snapshot(
    queue: &GtkWindowSizeQueue,
    notifier: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    size: (i32, i32),
) {
    if queue.push(size) {
        notify_pending_event(notifier);
    }
}

fn queue_window_focus_snapshot(
    queue: &GtkWindowFocusQueue,
    notifier: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    focused: bool,
) {
    if queue.push(focused) {
        notify_pending_event(notifier);
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

    fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("GtkEventQueue mutex should not be poisoned")
            .is_empty()
    }
}

/// Metadata for a ViewStackPage, stored in the host keyed by widget GObject pointer.
#[derive(Default, Clone)]
struct ViewStackPageMeta {
    name: String,
    title: String,
    icon_name: String,
}

#[derive(Clone)]
struct GridChildMeta {
    column: i32,
    row: i32,
    column_span: i32,
    row_span: i32,
}

impl Default for GridChildMeta {
    fn default() -> Self {
        Self {
            column: 0,
            row: 0,
            column_span: 1,
            row_span: 1,
        }
    }
}

#[derive(Clone, Default)]
struct TabPageMeta {
    title: String,
    needs_attention: bool,
    loading: bool,
}

struct FileChooserNativeState {
    native: gtk::FileChooserNative,
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
    event_notifier: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    /// Tracks the set of CSS class names that were last applied to each widget
    /// via the `cssClasses` property, keyed by the widget's GObject pointer.
    /// Needed so the classes can be cleanly replaced on each property update
    /// without accumulating stale class names.
    managed_css_classes: RefCell<BTreeMap<usize, BTreeSet<String>>>,
    /// Tracks the list of response IDs currently added to each AlertDialog,
    /// keyed by the widget's GObject pointer.
    alert_dialog_responses: RefCell<BTreeMap<usize, Vec<Box<str>>>>,
    /// Stores name/title/iconName for ViewStackPage widgets, keyed by GObject pointer.
    /// Read when the page is mounted into a ViewStack.
    view_stack_page_meta: RefCell<BTreeMap<usize, ViewStackPageMeta>>,
    /// Queue of `gtk.darkMode` state changes pushed by the `adw::StyleManager` notify
    /// signal.  Drained each tick and dispatched to the runtime.
    queued_dark_mode: Rc<GtkDarkModeQueue>,
    /// Whether the dark-mode watcher has been connected to `adw::StyleManager`.
    dark_mode_watcher_installed: bool,
    /// Queue of `clipboard.changed` text values pushed by the GDK clipboard watcher.
    /// Drained each tick and dispatched to the runtime.
    queued_clipboard: Rc<GtkClipboardQueue>,
    /// Whether the clipboard watcher has been connected to the GDK display clipboard.
    clipboard_watcher_installed: bool,
    grid_child_meta: RefCell<BTreeMap<usize, GridChildMeta>>,
    tab_page_meta: RefCell<BTreeMap<usize, TabPageMeta>>,
    file_chooser_states: Rc<RefCell<BTreeMap<usize, FileChooserNativeState>>>,
    queued_window_size: Rc<GtkWindowSizeQueue>,
    window_size_watcher_installed: bool,
    queued_window_focus: Rc<GtkWindowFocusQueue>,
    window_focus_watcher_installed: bool,
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
            event_notifier: Rc::new(RefCell::new(None)),
            managed_css_classes: RefCell::new(BTreeMap::new()),
            alert_dialog_responses: RefCell::new(BTreeMap::new()),
            view_stack_page_meta: RefCell::new(BTreeMap::new()),
            queued_dark_mode: Rc::new(GtkDarkModeQueue::default()),
            dark_mode_watcher_installed: false,
            queued_clipboard: Rc::new(GtkClipboardQueue::default()),
            clipboard_watcher_installed: false,
            grid_child_meta: RefCell::new(BTreeMap::new()),
            tab_page_meta: RefCell::new(BTreeMap::new()),
            file_chooser_states: Rc::new(RefCell::new(BTreeMap::new())),
            queued_window_size: Rc::new(GtkWindowSizeQueue::default()),
            window_size_watcher_installed: false,
            queued_window_focus: Rc::new(GtkWindowFocusQueue::default()),
            window_focus_watcher_installed: false,
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
        *self.event_notifier.borrow_mut() = notifier;
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

    /// Drains all pending dark-mode events queued by the `adw::StyleManager` watcher.
    /// Returns at most one value (the latest state) since the queue is coalescing.
    pub fn drain_dark_mode_events(&mut self) -> Vec<bool> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_dark_mode.drain()
    }

    /// Installs a one-time `adw::StyleManager` dark-notify watcher.  Idempotent: only
    /// the first call does anything; subsequent calls from repeated Window creation are
    /// no-ops.  Pushes the *current* dark state immediately so the source signal is
    /// populated before the first scheduler tick.
    fn setup_dark_mode_watcher(&mut self) {
        if self.dark_mode_watcher_installed {
            return;
        }
        self.dark_mode_watcher_installed = true;
        let queue = self.queued_dark_mode.clone();
        let notifier = self.event_notifier.clone();
        let style_manager = adw::StyleManager::default();
        // Push the current state immediately so the source is not uninitialized.
        queue.push(style_manager.is_dark());
        if let Some(n) = notifier.borrow().clone() {
            n();
        }
        style_manager.connect_dark_notify(move |mgr| {
            queue.push(mgr.is_dark());
            if let Some(n) = notifier.borrow().clone() {
                n();
            }
        });
    }

    /// Drains all pending clipboard events queued by the GDK clipboard watcher.
    /// Returns at most one value (the latest text) since the queue is coalescing.
    pub fn drain_clipboard_events(&mut self) -> Vec<String> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_clipboard.drain()
    }

    /// Installs a one-time GDK clipboard watcher.  Idempotent.
    /// Pushes the current clipboard text immediately at activation.
    fn setup_clipboard_watcher(&mut self) {
        if self.clipboard_watcher_installed {
            return;
        }
        self.clipboard_watcher_installed = true;
        let queue = self.queued_clipboard.clone();
        let notifier = self.event_notifier.clone();
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        // Read initial clipboard text to populate the source before first tick.
        {
            let queue_init = queue.clone();
            let notifier_init = notifier.clone();
            clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
                let text = result
                    .ok()
                    .flatten()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                queue_init.push(text);
                if let Some(n) = notifier_init.borrow().clone() {
                    n();
                }
            });
        }
        clipboard.connect_changed(move |cb| {
            let queue = queue.clone();
            let notifier = notifier.clone();
            cb.read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
                let text = result
                    .ok()
                    .flatten()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                queue.push(text);
                if let Some(n) = notifier.borrow().clone() {
                    n();
                }
            });
        });
    }

    pub fn drain_window_size_events(&mut self) -> Vec<(i32, i32)> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_window_size.drain()
    }

    pub fn drain_window_focus_events(&mut self) -> Vec<bool> {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_window_focus.drain()
    }

    pub fn has_pending_events(&self) -> bool {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        !self.queued_events.is_empty()
            || !self.queued_window_keys.is_empty()
            || !self.queued_dark_mode.is_empty()
            || !self.queued_clipboard.is_empty()
            || !self.queued_window_size.is_empty()
            || !self.queued_window_focus.is_empty()
    }

    fn setup_window_size_watcher(&mut self, window: &gtk::Window) {
        if self.window_size_watcher_installed {
            return;
        }
        self.window_size_watcher_installed = true;
        let queue = self.queued_window_size.clone();
        let notifier = self.event_notifier.clone();
        queue_window_size_snapshot(&queue, &notifier, (window.width(), window.height()));
        window.add_tick_callback(move |w, _| {
            queue_window_size_snapshot(&queue, &notifier, (w.width(), w.height()));
            glib::ControlFlow::Continue
        });
    }

    fn setup_window_focus_watcher(&mut self, window: &gtk::Window) {
        if self.window_focus_watcher_installed {
            return;
        }
        self.window_focus_watcher_installed = true;
        let queue = self.queued_window_focus.clone();
        let notifier = self.event_notifier.clone();
        queue_window_focus_snapshot(&queue, &notifier, window.is_active());
        window.connect_is_active_notify(move |w| {
            queue_window_focus_snapshot(&queue, &notifier, w.is_active());
        });
    }

    pub fn queue_window_key_event(&mut self, name: &str, repeated: bool) {
        GtkConcreteHost::<V>::assert_gtk_main_thread();
        self.queued_window_keys.push(GtkQueuedWindowKeyEvent {
            name: name.into(),
            repeated,
        });
        if let Some(notifier) = self.event_notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
            GtkConcreteWidgetKind::SpinButton => {
                gtk::SpinButton::with_range(0.0, 100.0, 1.0).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Scale => {
                gtk::Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0)
                    .upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Image => gtk::Image::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Spinner => gtk::Spinner::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ProgressBar => gtk::ProgressBar::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Revealer => gtk::Revealer::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Separator => {
                gtk::Separator::new(Orientation::Horizontal).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::StatusPage => adw::StatusPage::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Clamp => adw::Clamp::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Banner => adw::Banner::new("").upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ToolbarView => adw::ToolbarView::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ActionRow => adw::ActionRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ExpanderRow => adw::ExpanderRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::SwitchRow => adw::SwitchRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::SpinRow => {
                adw::SpinRow::with_range(0.0, 100.0, 1.0).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::EntryRow => adw::EntryRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ListBox => gtk::ListBox::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ListBoxRow => gtk::ListBoxRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ListView => {
                let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
                let selection = gtk::NoSelection::new(Some(store));
                let factory = create_collection_item_factory();
                gtk::ListView::new(Some(selection), Some(factory)).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::GridView => {
                let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
                let selection = gtk::NoSelection::new(Some(store));
                let factory = create_collection_item_factory();
                gtk::GridView::new(Some(selection), Some(factory)).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::DropDown => {
                gtk::DropDown::new(None::<gtk::StringList>, None::<gtk::Expression>)
                    .upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::SearchEntry => gtk::SearchEntry::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Expander => gtk::Expander::new(None).upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::NavigationView => {
                adw::NavigationView::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::NavigationPage => {
                let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
                adw::NavigationPage::new(&placeholder, "").upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::ToastOverlay => adw::ToastOverlay::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::PreferencesGroup => {
                adw::PreferencesGroup::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::PreferencesPage => {
                adw::PreferencesPage::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::PreferencesWindow => {
                adw::PreferencesWindow::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::ComboRow => adw::ComboRow::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::PasswordEntryRow => {
                adw::PasswordEntryRow::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Overlay => gtk::Overlay::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::MultilineEntry => gtk::TextView::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Picture => gtk::Picture::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::WebView => {
                let web_view = WebView::builder()
                    .network_session(&NetworkSession::new_ephemeral())
                    .default_content_security_policy(AIVI_WEB_VIEW_DEFAULT_CSP)
                    .build();
                configure_aivi_web_view(&web_view);
                web_view.upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::ViewStack => adw::ViewStack::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::ViewStackPage => adw::Bin::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::AlertDialog => {
                adw::MessageDialog::new(None::<&gtk::Window>, None, None).upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Calendar => gtk::Calendar::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::FlowBox => gtk::FlowBox::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::FlowBoxChild => gtk::FlowBoxChild::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::MenuButton => gtk::MenuButton::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Popover => gtk::Popover::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::CenterBox => gtk::CenterBox::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::AboutDialog => adw::AboutDialog::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::SplitButton => adw::SplitButton::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::NavigationSplitView => {
                adw::NavigationSplitView::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::OverlaySplitView => {
                adw::OverlaySplitView::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::TabView => adw::TabView::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::TabPage => {
                let bin = adw::Bin::new();
                let key = bin.as_ptr() as usize;
                self.tab_page_meta
                    .borrow_mut()
                    .insert(key, TabPageMeta::default());
                bin.upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::TabBar => adw::TabBar::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::Carousel => adw::Carousel::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::CarouselIndicatorDots => {
                adw::CarouselIndicatorDots::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::CarouselIndicatorLines => {
                adw::CarouselIndicatorLines::new().upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::Grid => gtk::Grid::new().upcast::<gtk::Widget>(),
            GtkConcreteWidgetKind::GridChild => {
                let bin = adw::Bin::new();
                let key = bin.as_ptr() as usize;
                self.grid_child_meta
                    .borrow_mut()
                    .insert(key, GridChildMeta::default());
                bin.upcast::<gtk::Widget>()
            }
            GtkConcreteWidgetKind::FileDialog => {
                let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
                placeholder.set_visible(false);
                let native = gtk::FileChooserNative::new(
                    None,
                    None::<&gtk::Window>,
                    gtk::FileChooserAction::Open,
                    None,
                    None,
                );
                let key = placeholder.as_ptr() as usize;
                self.file_chooser_states
                    .borrow_mut()
                    .insert(key, FileChooserNativeState { native });
                placeholder.upcast::<gtk::Widget>()
            }
        };
        ensure_aivi_widget_styles();
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

    fn lookup_property(
        &self,
        schema: &'static GtkWidgetSchema,
        property: &str,
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

    /// If `widget` (an `adw::Bin` GridChild) is already attached to a `gtk::Grid`
    /// parent, remove it and re-attach it at its current position from the meta map.
    ///
    /// This is required because the hydration plan sets column/row properties AFTER
    /// `update_each_keyed` has already called `grid.attach()` with the default (0, 0)
    /// position.  Without re-attaching, every tile would sit on top of each other at
    /// the origin.
    fn reattach_grid_child_if_mounted(&self, widget: &gtk::Widget, key: usize) {
        let Some(parent) = widget.parent() else {
            return;
        };
        let Ok(grid) = parent.downcast::<gtk::Grid>() else {
            return;
        };
        let meta = self.grid_child_meta.borrow();
        let meta = meta.get(&key).cloned().unwrap_or_default();
        grid.remove(widget);
        grid.attach(
            widget,
            meta.column,
            meta.row,
            meta.column_span.max(1),
            meta.row_span.max(1),
        );
    }

    fn with_blocked_widget_events<T>(
        &self,
        widget: &gtk::Widget,
        f: impl FnOnce() -> Result<T, GtkConcreteHostError>,
    ) -> Result<T, GtkConcreteHostError> {
        let relevant: Vec<&MountedEvent> = self
            .events
            .values()
            .filter(|mounted| mounted.widget.as_ptr() == widget.as_ptr())
            .collect();
        for mounted in &relevant {
            mounted.signal_object.block_signal(&mounted.signal);
        }
        let result = f();
        for mounted in relevant.iter().rev() {
            mounted.signal_object.unblock_signal(&mounted.signal);
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
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Focusable) => {
                widget.set_focusable(value)
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Hexpand) => widget.set_hexpand(value),
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Vexpand) => widget.set_vexpand(value),
            GtkPropertySetter::Bool(GtkBoolPropertySetter::AnimateOpacity) => {
                if value {
                    widget.add_css_class("aivi-animate-opacity");
                } else {
                    widget.remove_css_class("aivi-animate-opacity");
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::Monospace) => {
                if value {
                    widget.add_css_class("monospace");
                } else {
                    widget.remove_css_class("monospace");
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonCompact) => {
                if value {
                    widget.add_css_class("aivi-compact-button");
                } else {
                    widget.remove_css_class("aivi-compact-button");
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonHasFrame) => {
                widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Button",
                    })?
                    .set_has_frame(value);
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
            GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinButtonWrap) => {
                widget
                    .clone()
                    .downcast::<gtk::SpinButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    })?
                    .set_wrap(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinButtonNumeric) => {
                widget
                    .clone()
                    .downcast::<gtk::SpinButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    })?
                    .set_numeric(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ScaleDrawValue) => {
                widget
                    .clone()
                    .downcast::<gtk::Scale>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    })?
                    .set_draw_value(value);
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
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowResizable) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_resizable(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowModal) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_modal(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelWrap) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_wrap(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelSelectable) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_selectable(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelUseMarkup) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_use_markup(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::EntryVisibility) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_visibility(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ScrolledWindowPropagateNaturalWidth) => {
                widget
                    .clone()
                    .downcast::<gtk::ScrolledWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ScrolledWindow",
                    })?
                    .set_propagate_natural_width(value);
            }
            GtkPropertySetter::Bool(
                GtkBoolPropertySetter::ScrolledWindowPropagateNaturalHeight,
            ) => {
                widget
                    .clone()
                    .downcast::<gtk::ScrolledWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ScrolledWindow",
                    })?
                    .set_propagate_natural_height(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ProgressBarShowText) => {
                widget
                    .clone()
                    .downcast::<gtk::ProgressBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ProgressBar",
                    })?
                    .set_show_text(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::BoxHomogeneous) => {
                widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Box",
                    })?
                    .set_homogeneous(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::BannerRevealed) => {
                widget
                    .clone()
                    .downcast::<adw::Banner>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Banner",
                    })?
                    .set_revealed(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ExpanderRowExpanded) => {
                widget
                    .clone()
                    .downcast::<adw::ExpanderRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ExpanderRow",
                    })?
                    .set_expanded(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::SwitchRowActive) => {
                widget
                    .clone()
                    .downcast::<adw::SwitchRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SwitchRow",
                    })?
                    .set_active(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ListBoxRowActivatable) => {
                widget
                    .clone()
                    .downcast::<gtk::ListBoxRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListBoxRow",
                    })?
                    .set_activatable(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ExpanderExpanded) => {
                widget
                    .clone()
                    .downcast::<gtk::Expander>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Expander",
                    })?
                    .set_expanded(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::PreferencesWindowSearchEnabled) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesWindow",
                    })?
                    .set_search_enabled(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonUseUnderline) => {
                widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Button",
                    })?
                    .set_use_underline(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::MultilineEntryEditable) => {
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_editable(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::MultilineEntryMonospace) => {
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_monospace(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::PictureCanShrink) => {
                widget
                    .clone()
                    .downcast::<gtk::Picture>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Picture",
                    })?
                    .set_can_shrink(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowMaximized) => {
                let window = widget.clone().downcast::<gtk::Window>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    }
                })?;
                if value {
                    window.maximize();
                } else {
                    window.unmaximize();
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowFullscreen) => {
                let window = widget.clone().downcast::<gtk::Window>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    }
                })?;
                if value {
                    window.fullscreen();
                } else {
                    window.unfullscreen();
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowDecorated) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_decorated(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowHideOnClose) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_hide_on_close(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonReceivesDefault) => {
                widget.set_receives_default(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelSingleLineMode) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_single_line_mode(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::MenuButtonActive) => {
                let mb = widget.clone().downcast::<gtk::MenuButton>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::MenuButton",
                    }
                })?;
                mb.set_property("active", value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::MenuButtonUseUnderline) => {
                widget
                    .clone()
                    .downcast::<gtk::MenuButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::MenuButton",
                    })?
                    .set_use_underline(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::PopoverAutohide) => {
                widget
                    .clone()
                    .downcast::<gtk::Popover>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Popover",
                    })?
                    .set_autohide(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::PopoverHasArrow) => {
                widget
                    .clone()
                    .downcast::<gtk::Popover>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Popover",
                    })?
                    .set_has_arrow(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::AboutDialogVisible) => {
                if let Ok(dialog) = widget.clone().downcast::<adw::AboutDialog>() {
                    if value {
                        dialog.present(None::<&gtk::Window>);
                    }
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::FileDialogVisible) => {
                if value {
                    let key = widget.as_ptr() as usize;
                    if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                        state.native.show();
                    }
                }
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::NavigationSplitViewShowContent) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationSplitView",
                    })?
                    .set_show_content(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::OverlaySplitViewShowSidebar) => {
                widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_show_sidebar(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::TabPageNeedsAttention) => {
                let key = widget.as_ptr() as usize;
                self.tab_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .needs_attention = value;
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::TabPageLoading) => {
                let key = widget.as_ptr() as usize;
                self.tab_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .loading = value;
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::TabBarAutohide) => {
                widget
                    .clone()
                    .downcast::<adw::TabBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::TabBar",
                    })?
                    .set_autohide(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::TabBarExpandTabs) => {
                widget
                    .clone()
                    .downcast::<adw::TabBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::TabBar",
                    })?
                    .set_expand_tabs(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::CarouselInteractive) => {
                widget
                    .clone()
                    .downcast::<adw::Carousel>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Carousel",
                    })?
                    .set_interactive(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::GridRowHomogeneous) => {
                widget
                    .clone()
                    .downcast::<gtk::Grid>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Grid",
                    })?
                    .set_row_homogeneous(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::GridColumnHomogeneous) => {
                widget
                    .clone()
                    .downcast::<gtk::Grid>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Grid",
                    })?
                    .set_column_homogeneous(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ListBoxShowSeparators) => {
                widget
                    .clone()
                    .downcast::<gtk::ListBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListBox",
                    })?
                    .set_show_separators(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewShowSeparators) => {
                widget
                    .clone()
                    .downcast::<gtk::ListView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListView",
                    })?
                    .set_show_separators(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewEnableRubberband) => {
                widget
                    .clone()
                    .downcast::<gtk::ListView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListView",
                    })?
                    .set_enable_rubberband(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewSingleClickActivate) => {
                widget
                    .clone()
                    .downcast::<gtk::ListView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListView",
                    })?
                    .set_single_click_activate(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::GridViewEnableRubberband) => {
                widget
                    .clone()
                    .downcast::<gtk::GridView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::GridView",
                    })?
                    .set_enable_rubberband(value);
            }
            GtkPropertySetter::Bool(GtkBoolPropertySetter::GridViewSingleClickActivate) => {
                widget
                    .clone()
                    .downcast::<gtk::GridView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::GridView",
                    })?
                    .set_single_click_activate(value);
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
            GtkPropertySetter::Text(GtkTextPropertySetter::WindowColorScheme) => {
                let scheme = match value {
                    "force-light" => adw::ColorScheme::ForceLight,
                    "prefer-light" => adw::ColorScheme::PreferLight,
                    "force-dark" => adw::ColorScheme::ForceDark,
                    "prefer-dark" => adw::ColorScheme::PreferDark,
                    _ => adw::ColorScheme::Default,
                };
                adw::StyleManager::default().set_color_scheme(scheme);
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
            GtkPropertySetter::Text(GtkTextPropertySetter::ScaleOrientation) => {
                let orientation = parse_orientation(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "Vertical or Horizontal")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Scale>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
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
            GtkPropertySetter::Text(GtkTextPropertySetter::Halign) => {
                let align = parse_align(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid Align value")
                })?;
                widget.set_halign(align);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::Valign) => {
                let align = parse_align(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid Align value")
                })?;
                widget.set_valign(align);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::Tooltip) => {
                if value.is_empty() {
                    widget.set_tooltip_text(None);
                } else {
                    widget.set_tooltip_text(Some(value));
                }
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::CssClasses) => {
                let key = widget.as_ptr() as usize;
                let mut map = self.managed_css_classes.borrow_mut();
                if let Some(previous) = map.get(&key) {
                    for class in previous {
                        widget.remove_css_class(class.as_str());
                    }
                }
                let mut next_set = BTreeSet::new();
                if !value.is_empty() {
                    for class in value.split_ascii_whitespace() {
                        widget.add_css_class(class);
                        next_set.insert(class.to_owned());
                    }
                }
                map.insert(key, next_set);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::LabelWrapMode) => {
                let mode = parse_wrap_mode(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid WrapMode value")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_wrap_mode(mode);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::LabelJustify) => {
                let justification = parse_justification(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid Justification value")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_justify(justification);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::LabelEllipsize) => {
                let mode = parse_ellipsize(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid EllipsizeMode value")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_ellipsize(mode);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntryInputPurpose) => {
                let purpose = parse_input_purpose(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid InputPurpose value")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_input_purpose(purpose);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ScrolledWindowHPolicy) => {
                let policy = parse_policy(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid PolicyType value")
                })?;
                let sw = widget
                    .clone()
                    .downcast::<gtk::ScrolledWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ScrolledWindow",
                    })?;
                sw.set_policy(policy, sw.vscrollbar_policy());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ScrolledWindowVPolicy) => {
                let policy = parse_policy(value).ok_or_else(|| {
                    self.invalid_property_value(schema, property, "valid PolicyType value")
                })?;
                let sw = widget
                    .clone()
                    .downcast::<gtk::ScrolledWindow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ScrolledWindow",
                    })?;
                sw.set_policy(sw.hscrollbar_policy(), policy);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ImageFile) => {
                widget
                    .clone()
                    .downcast::<gtk::Image>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Image",
                    })?
                    .set_from_file(if value.is_empty() {
                        None::<&str>
                    } else {
                        Some(value)
                    });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageTitle) => {
                widget
                    .clone()
                    .downcast::<adw::StatusPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::StatusPage",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageDescription) => {
                widget
                    .clone()
                    .downcast::<adw::StatusPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::StatusPage",
                    })?
                    .set_description(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageIconName) => {
                let icon = if value.is_empty() { None } else { Some(value) };
                widget
                    .clone()
                    .downcast::<adw::StatusPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::StatusPage",
                    })?
                    .set_icon_name(icon);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::BannerTitle) => {
                widget
                    .clone()
                    .downcast::<adw::Banner>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Banner",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::BannerButtonLabel) => {
                widget
                    .clone()
                    .downcast::<adw::Banner>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Banner",
                    })?
                    .set_button_label(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AdwPreferencesRowTitle) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesRow",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AdwActionRowSubtitle) => {
                widget
                    .clone()
                    .downcast::<adw::ActionRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ActionRow",
                    })?
                    .set_subtitle(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AdwExpanderRowSubtitle) => {
                widget
                    .clone()
                    .downcast::<adw::ExpanderRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ExpanderRow",
                    })?
                    .set_subtitle(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntryRowText) => {
                widget
                    .clone()
                    .downcast::<adw::EntryRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::EntryRow",
                    })?
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ListBoxSelectionMode) => {
                use gtk::SelectionMode;
                let mode = match value {
                    "None" => SelectionMode::None,
                    "Single" => SelectionMode::Single,
                    "Browse" => SelectionMode::Browse,
                    "Multiple" => SelectionMode::Multiple,
                    _ => {
                        return Err(self.invalid_property_value(
                            schema,
                            property,
                            "valid SelectionMode value (None, Single, Browse, Multiple)",
                        ));
                    }
                };
                widget
                    .clone()
                    .downcast::<gtk::ListBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::ListBox",
                    })?
                    .set_selection_mode(mode);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::DropDownItems) => {
                let items: Vec<&str> = if value.is_empty() {
                    vec![]
                } else {
                    value.split(',').map(str::trim).collect()
                };
                let model = gtk::StringList::new(&items);
                widget
                    .clone()
                    .downcast::<gtk::DropDown>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::DropDown",
                    })?
                    .set_model(Some(&model));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::SearchEntryText) => {
                widget
                    .clone()
                    .downcast::<gtk::SearchEntry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SearchEntry",
                    })?
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::SearchEntryPlaceholder) => {
                widget
                    .clone()
                    .downcast::<gtk::SearchEntry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SearchEntry",
                    })?
                    .set_placeholder_text(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ExpanderLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::Expander>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Expander",
                    })?
                    .set_label(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::NavigationPageTitle) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationPage",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::NavigationPageTag) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationPage",
                    })?
                    .set_tag(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesGroupTitle) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesGroup>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesGroup",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesGroupDescription) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesGroup>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesGroup",
                    })?
                    .set_description(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesPageTitle) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesPage",
                    })?
                    .set_title(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesPageIconName) => {
                widget
                    .clone()
                    .downcast::<adw::PreferencesPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PreferencesPage",
                    })?
                    .set_icon_name(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ComboRowItems) => {
                let row = widget.clone().downcast::<adw::ComboRow>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ComboRow",
                    }
                })?;
                let items: Vec<&str> = if value.is_empty() {
                    vec![]
                } else {
                    value.split(',').map(str::trim).collect()
                };
                let model = gtk::StringList::new(&items);
                row.set_model(Some(&model));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PasswordEntryRowText) => {
                widget
                    .clone()
                    .downcast::<adw::PasswordEntryRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::PasswordEntryRow",
                    })?
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ButtonIconName) => {
                widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Button",
                    })?
                    .set_icon_name(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::MultilineEntryText) => {
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .buffer()
                    .set_text(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::MultilineEntryWrapMode) => {
                let wrap_mode = match value {
                    "None" => gtk::WrapMode::None,
                    "Char" => gtk::WrapMode::Char,
                    "Word" => gtk::WrapMode::Word,
                    "WordChar" => gtk::WrapMode::WordChar,
                    _ => {
                        return Err(self.invalid_property_value(
                            schema,
                            property,
                            "text naming a valid WrapMode value (None, Char, Word, WordChar)",
                        ));
                    }
                };
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_wrap_mode(wrap_mode);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PictureFilename) => {
                widget
                    .clone()
                    .downcast::<gtk::Picture>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Picture",
                    })?
                    .set_filename(if value.is_empty() {
                        None::<&str>
                    } else {
                        Some(value)
                    });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PictureResource) => {
                widget
                    .clone()
                    .downcast::<gtk::Picture>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Picture",
                    })?
                    .set_resource(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PictureContentFit) => {
                let fit = match value {
                    "fill" => gtk::ContentFit::Fill,
                    "cover" => gtk::ContentFit::Cover,
                    "scale-down" => gtk::ContentFit::ScaleDown,
                    _ => gtk::ContentFit::Contain,
                };
                widget
                    .clone()
                    .downcast::<gtk::Picture>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Picture",
                    })?
                    .set_content_fit(fit);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::PictureAltText) => {
                widget
                    .clone()
                    .downcast::<gtk::Picture>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Picture",
                    })?
                    .set_alternative_text(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::WebViewHtml) => {
                widget
                    .clone()
                    .downcast::<WebView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "webkit6::WebView",
                    })?
                    .load_html(value, None);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackVisibleChild) => {
                widget
                    .clone()
                    .downcast::<adw::ViewStack>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ViewStack",
                    })?
                    .set_visible_child_name(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageName) => {
                let key = widget.as_ptr() as usize;
                self.view_stack_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .name = value.to_owned();
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageTitle) => {
                let key = widget.as_ptr() as usize;
                self.view_stack_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .title = value.to_owned();
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageIconName) => {
                let key = widget.as_ptr() as usize;
                self.view_stack_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .icon_name = value.to_owned();
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogHeading) => {
                widget
                    .clone()
                    .downcast::<adw::MessageDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::MessageDialog",
                    })?
                    .set_heading(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogBody) => {
                widget
                    .clone()
                    .downcast::<adw::MessageDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::MessageDialog",
                    })?
                    .set_body(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogDefaultResponse) => {
                widget
                    .clone()
                    .downcast::<adw::MessageDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::MessageDialog",
                    })?
                    .set_default_response(if value.is_empty() { None } else { Some(value) });
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogCloseResponse) => {
                widget
                    .clone()
                    .downcast::<adw::MessageDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::MessageDialog",
                    })?
                    .set_close_response(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogResponses) => {
                let dialog = widget
                    .clone()
                    .downcast::<adw::MessageDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::MessageDialog",
                    })?;
                let key = widget.as_ptr() as usize;
                let mut tracked = self.alert_dialog_responses.borrow_mut();
                let current_ids = tracked.entry(key).or_default();

                // Parse "id:Label:appearance|id2:Label2" format.
                let new_responses: Vec<(Box<str>, Box<str>, adw::ResponseAppearance)> = value
                    .split('|')
                    .filter(|s| !s.is_empty())
                    .map(|entry| {
                        let mut parts = entry.splitn(3, ':');
                        let id = parts.next().unwrap_or("").trim();
                        let label = parts.next().unwrap_or(id).trim();
                        let appearance_str = parts.next().unwrap_or("").trim();
                        let appearance = match appearance_str {
                            "suggested" => adw::ResponseAppearance::Suggested,
                            "destructive" => adw::ResponseAppearance::Destructive,
                            _ => adw::ResponseAppearance::Default,
                        };
                        (id.into(), label.into(), appearance)
                    })
                    .collect();

                let new_ids: Vec<Box<str>> =
                    new_responses.iter().map(|(id, _, _)| id.clone()).collect();

                // Remove stale responses (requires adw v1.5).
                let stale: Vec<Box<str>> = current_ids
                    .iter()
                    .filter(|id| !new_ids.contains(id))
                    .cloned()
                    .collect();
                for id in &stale {
                    dialog.remove_response(id.as_ref());
                }

                // Add new responses and update appearance.
                for (id, label, appearance) in &new_responses {
                    if !current_ids.contains(id) {
                        dialog.add_response(id.as_ref(), label.as_ref());
                    }
                    dialog.set_response_appearance(id.as_ref(), *appearance);
                }

                *current_ids = new_ids;
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::HeaderBarDecorationLayout) => {
                widget
                    .clone()
                    .downcast::<gtk::HeaderBar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::HeaderBar",
                    })?
                    .set_decoration_layout(Some(value));
                return Ok(());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::ScaleValuePos) => {
                let pos = match value {
                    "Top" => gtk::PositionType::Top,
                    "Bottom" => gtk::PositionType::Bottom,
                    "Left" => gtk::PositionType::Left,
                    "Right" => gtk::PositionType::Right,
                    _ => {
                        return Err(self.invalid_property_value(
                            schema,
                            property,
                            "Top, Bottom, Left, or Right",
                        ));
                    }
                };
                widget
                    .clone()
                    .downcast::<gtk::Scale>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    })?
                    .set_value_pos(pos);
                return Ok(());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FlowBoxSelectionMode) => {
                let mode = match value {
                    "None" => gtk::SelectionMode::None,
                    "Single" => gtk::SelectionMode::Single,
                    "Browse" => gtk::SelectionMode::Browse,
                    "Multiple" => gtk::SelectionMode::Multiple,
                    _ => {
                        return Err(self.invalid_property_value(
                            schema,
                            property,
                            "None, Single, Browse, or Multiple",
                        ));
                    }
                };
                widget
                    .clone()
                    .downcast::<gtk::FlowBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::FlowBox",
                    })?
                    .set_selection_mode(mode);
                return Ok(());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::MenuButtonLabel) => {
                widget
                    .clone()
                    .downcast::<gtk::MenuButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::MenuButton",
                    })?
                    .set_label(value);
                return Ok(());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::MenuButtonIconName) => {
                widget
                    .clone()
                    .downcast::<gtk::MenuButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::MenuButton",
                    })?
                    .set_icon_name(value);
                return Ok(());
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogAppName) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_application_name(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogVersion) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_version(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogDeveloperName) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_developer_name(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogComments) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_comments(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogWebsite) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_website(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogIssueUrl) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_issue_url(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogLicenseType) => {
                let license = match value {
                    "Agpl30" => gtk::License::Agpl30,
                    "Agpl30Only" => gtk::License::Agpl30Only,
                    "Apache20" => gtk::License::Apache20,
                    "Artistic" => gtk::License::Artistic,
                    "Bsd" => gtk::License::Bsd,
                    "Gpl20" => gtk::License::Gpl20,
                    "Gpl20Only" => gtk::License::Gpl20Only,
                    "Gpl30" => gtk::License::Gpl30,
                    "Gpl30Only" => gtk::License::Gpl30Only,
                    "Lgpl21" => gtk::License::Lgpl21,
                    "Lgpl21Only" => gtk::License::Lgpl21Only,
                    "Lgpl30" => gtk::License::Lgpl30,
                    "Lgpl30Only" => gtk::License::Lgpl30Only,
                    "Mit" => gtk::License::MitX11,
                    "MplTwo" => gtk::License::Mpl20,
                    "Custom" => gtk::License::Custom,
                    _ => gtk::License::Unknown,
                };
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_license_type(license);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogApplicationIcon) => {
                widget
                    .clone()
                    .downcast::<adw::AboutDialog>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::AboutDialog",
                    })?
                    .set_application_icon(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::TabPageTitle) => {
                let key = widget.as_ptr() as usize;
                self.tab_page_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .title = value.to_owned();
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::SplitButtonLabel) => {
                widget
                    .clone()
                    .downcast::<adw::SplitButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SplitButton",
                    })?
                    .set_label(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::SplitButtonIconName) => {
                widget
                    .clone()
                    .downcast::<adw::SplitButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SplitButton",
                    })?
                    .set_icon_name(value);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::NavigationSplitViewSidebarPosition) => {
                // set_sidebar_position requires libadwaita v1.7 which is not yet enabled; no-op.
                let _ = value;
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::OverlaySplitViewSidebarPosition) => {
                let pack_type = if value == "End" {
                    gtk::PackType::End
                } else {
                    gtk::PackType::Start
                };
                widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_sidebar_position(pack_type);
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogTitle) => {
                let key = widget.as_ptr() as usize;
                if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                    state.native.set_title(value);
                }
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogMode) => {
                let action = match value {
                    "Save" => gtk::FileChooserAction::Save,
                    "OpenMultiple" => gtk::FileChooserAction::Open,
                    "SelectFolder" => gtk::FileChooserAction::SelectFolder,
                    _ => gtk::FileChooserAction::Open,
                };
                let key = widget.as_ptr() as usize;
                if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                    state.native.set_action(action);
                }
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogAcceptLabel) => {
                let key = widget.as_ptr() as usize;
                if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                    state.native.set_accept_label(Some(value));
                }
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogCancelLabel) => {
                let key = widget.as_ptr() as usize;
                if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                    state.native.set_cancel_label(Some(value));
                }
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntryPrimaryIconName) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_primary_icon_name(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::EntrySecondaryIconName) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_secondary_icon_name(Some(value));
            }
            GtkPropertySetter::Text(GtkTextPropertySetter::HeaderBarCenteringPolicy) => {
                // gtk::HeaderBar in GTK4 does not expose centering-policy in Rust bindings; no-op.
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
            GtkPropertySetter::I64(GtkI64PropertySetter::WidthRequest) => {
                let size = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget.set_width_request(size);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::HeightRequest) => {
                let size = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget.set_height_request(size);
                Ok(())
            }
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
            GtkPropertySetter::I64(GtkI64PropertySetter::SpinButtonDigits) => {
                let digits = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::SpinButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    })?
                    .set_digits(digits);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::ScaleDigits) => {
                let digits = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Scale>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    })?
                    .set_digits(digits);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MarginStart) => {
                widget.set_margin_start(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MarginEnd) => {
                widget.set_margin_end(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MarginTop) => {
                widget.set_margin_top(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MarginBottom) => {
                widget.set_margin_bottom(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::WindowDefaultWidth) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_default_width(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::WindowDefaultHeight) => {
                widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_default_height(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::LabelMaxWidthChars) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_max_width_chars(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::EntryMaxLength) => {
                widget
                    .clone()
                    .downcast::<gtk::Entry>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Entry",
                    })?
                    .set_max_length(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::ClampMaximumSize) => {
                widget
                    .clone()
                    .downcast::<adw::Clamp>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Clamp",
                    })?
                    .set_maximum_size(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::ClampTighteningThreshold) => {
                widget
                    .clone()
                    .downcast::<adw::Clamp>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Clamp",
                    })?
                    .set_tightening_threshold(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::DropDownSelected) => {
                let position = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::DropDown>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::DropDown",
                    })?
                    .set_selected(position);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::ComboRowSelected) => {
                let position = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<adw::ComboRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::ComboRow",
                    })?
                    .set_selected(position);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::LabelLines) => {
                let lines = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_lines(lines);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryTopMargin) => {
                let margin = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_top_margin(margin);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryBottomMargin) => {
                let margin = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_bottom_margin(margin);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryLeftMargin) => {
                let margin = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_left_margin(margin);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryRightMargin) => {
                let margin = i32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "signed 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::TextView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    })?
                    .set_right_margin(margin);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::LabelWidthChars) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_width_chars(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::CalendarYear) => {
                widget
                    .clone()
                    .downcast::<gtk::Calendar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Calendar",
                    })?
                    .set_year(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::CalendarMonth) => {
                widget
                    .clone()
                    .downcast::<gtk::Calendar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Calendar",
                    })?
                    .set_month(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::CalendarDay) => {
                widget
                    .clone()
                    .downcast::<gtk::Calendar>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Calendar",
                    })?
                    .set_day(value as i32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::FlowBoxRowSpacing) => {
                let spacing = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::FlowBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::FlowBox",
                    })?
                    .set_row_spacing(spacing);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::FlowBoxColumnSpacing) => {
                let spacing = u32::try_from(value).map_err(|_| {
                    self.invalid_property_value(schema, property, "non-negative 32-bit integer")
                })?;
                widget
                    .clone()
                    .downcast::<gtk::FlowBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::FlowBox",
                    })?
                    .set_column_spacing(spacing);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridRowSpacing) => {
                widget
                    .clone()
                    .downcast::<gtk::Grid>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Grid",
                    })?
                    .set_row_spacing(value.max(0) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridColumnSpacing) => {
                widget
                    .clone()
                    .downcast::<gtk::Grid>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Grid",
                    })?
                    .set_column_spacing(value.max(0) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridViewMinColumns) => {
                widget
                    .clone()
                    .downcast::<gtk::GridView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::GridView",
                    })?
                    .set_min_columns(value.max(1) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridViewMaxColumns) => {
                widget
                    .clone()
                    .downcast::<gtk::GridView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::GridView",
                    })?
                    .set_max_columns(value.max(1) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridChildColumn) => {
                let key = widget.as_ptr() as usize;
                let prev = self.grid_child_meta.borrow().get(&key).map(|m| m.column);
                let new_val = value as i32;
                self.grid_child_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .column = new_val;
                if prev != Some(new_val) {
                    self.reattach_grid_child_if_mounted(widget, key);
                }
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridChildRow) => {
                let key = widget.as_ptr() as usize;
                let prev = self.grid_child_meta.borrow().get(&key).map(|m| m.row);
                let new_val = value as i32;
                self.grid_child_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .row = new_val;
                if prev != Some(new_val) {
                    self.reattach_grid_child_if_mounted(widget, key);
                }
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridChildColumnSpan) => {
                let key = widget.as_ptr() as usize;
                let prev = self
                    .grid_child_meta
                    .borrow()
                    .get(&key)
                    .map(|m| m.column_span);
                let new_val = value.max(1) as i32;
                self.grid_child_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .column_span = new_val;
                if prev != Some(new_val) {
                    self.reattach_grid_child_if_mounted(widget, key);
                }
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::GridChildRowSpan) => {
                let key = widget.as_ptr() as usize;
                let prev = self.grid_child_meta.borrow().get(&key).map(|m| m.row_span);
                let new_val = value.max(1) as i32;
                self.grid_child_meta
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .row_span = new_val;
                if prev != Some(new_val) {
                    self.reattach_grid_child_if_mounted(widget, key);
                }
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::CarouselSpacing) => {
                widget
                    .clone()
                    .downcast::<adw::Carousel>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Carousel",
                    })?
                    .set_spacing(value.max(0) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::CarouselRevealDuration) => {
                widget
                    .clone()
                    .downcast::<adw::Carousel>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::Carousel",
                    })?
                    .set_reveal_duration(value.max(0) as u32);
                Ok(())
            }
            GtkPropertySetter::I64(GtkI64PropertySetter::TabViewSelectedPage) => {
                let tab_view = widget.clone().downcast::<adw::TabView>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::TabView",
                    }
                })?;
                let n = tab_view.n_pages();
                let idx = value as i32;
                if idx >= 0 && idx < n {
                    if let Some(page) = tab_view.pages().item(idx as u32) {
                        if let Ok(tab_page) = page.downcast::<adw::TabPage>() {
                            tab_view.set_selected_page(&tab_page);
                        }
                    }
                }
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
            GtkPropertySetter::F64(GtkF64PropertySetter::WidgetOpacity) => {
                widget.set_opacity(value.clamp(0.0, 1.0));
                Ok(())
            }
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
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonValue) => {
                widget
                    .clone()
                    .downcast::<gtk::SpinButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    })?
                    .set_value(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonMin) => {
                let spin = widget.clone().downcast::<gtk::SpinButton>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    }
                })?;
                spin.adjustment().set_lower(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonMax) => {
                let spin = widget.clone().downcast::<gtk::SpinButton>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    }
                })?;
                spin.adjustment().set_upper(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonStep) => {
                let spin = widget.clone().downcast::<gtk::SpinButton>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::SpinButton",
                    }
                })?;
                spin.adjustment().set_step_increment(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::ScaleValue) => {
                widget
                    .clone()
                    .downcast::<gtk::Scale>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    })?
                    .set_value(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::ScaleMin) => {
                let scale = widget.clone().downcast::<gtk::Scale>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    }
                })?;
                scale.adjustment().set_lower(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::ScaleMax) => {
                let scale = widget.clone().downcast::<gtk::Scale>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    }
                })?;
                scale.adjustment().set_upper(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::ScaleStep) => {
                let scale = widget.clone().downcast::<gtk::Scale>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    }
                })?;
                scale.adjustment().set_step_increment(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowValue) => {
                widget
                    .clone()
                    .downcast::<adw::SpinRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SpinRow",
                    })?
                    .set_value(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowMin) => {
                let spin = widget.clone().downcast::<adw::SpinRow>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SpinRow",
                    }
                })?;
                spin.adjustment().set_lower(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowMax) => {
                let spin = widget.clone().downcast::<adw::SpinRow>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SpinRow",
                    }
                })?;
                spin.adjustment().set_upper(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowStep) => {
                let spin = widget.clone().downcast::<adw::SpinRow>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::SpinRow",
                    }
                })?;
                spin.adjustment().set_step_increment(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::ScaleFillLevel) => {
                let scale = widget.clone().downcast::<gtk::Scale>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Scale",
                    }
                })?;
                scale.set_show_fill_level(true);
                scale.set_fill_level(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::LabelXalign) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_xalign(value as f32);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::LabelYalign) => {
                widget
                    .clone()
                    .downcast::<gtk::Label>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::Label",
                    })?
                    .set_yalign(value as f32);
                Ok(())
            }
            GtkPropertySetter::F64(
                GtkF64PropertySetter::NavigationSplitViewSidebarWidthFraction,
            ) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationSplitView",
                    })?
                    .set_sidebar_width_fraction(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::NavigationSplitViewMinSidebarWidth) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationSplitView",
                    })?
                    .set_min_sidebar_width(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::NavigationSplitViewMaxSidebarWidth) => {
                widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::NavigationSplitView",
                    })?
                    .set_max_sidebar_width(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewSidebarWidthFraction) => {
                widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_sidebar_width_fraction(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewMinSidebarWidth) => {
                widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_min_sidebar_width(value);
                Ok(())
            }
            GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewMaxSidebarWidth) => {
                widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_max_sidebar_width(value);
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
            GtkChildMountRoute::ToolbarViewTop => {
                for child in previous {
                    child.unparent();
                }
                let toolbar_view = parent_widget
                    .clone()
                    .downcast::<adw::ToolbarView>()
                    .expect("toolbar view widget should downcast");
                for child in next {
                    toolbar_view.add_top_bar(child);
                }
            }
            GtkChildMountRoute::ToolbarViewBottom => {
                for child in previous {
                    child.unparent();
                }
                let toolbar_view = parent_widget
                    .clone()
                    .downcast::<adw::ToolbarView>()
                    .expect("toolbar view widget should downcast");
                for child in next {
                    toolbar_view.add_bottom_bar(child);
                }
            }
            GtkChildMountRoute::ActionRowSuffix => {
                for child in previous {
                    child.unparent();
                }
                let row = parent_widget
                    .clone()
                    .downcast::<adw::ActionRow>()
                    .expect("action row widget should downcast");
                for child in next {
                    row.add_suffix(child);
                }
            }
            GtkChildMountRoute::ActionRowPrefix => {
                for child in previous {
                    child.unparent();
                }
                let row = parent_widget
                    .clone()
                    .downcast::<adw::ActionRow>()
                    .expect("action row widget should downcast");
                for child in next {
                    row.add_prefix(child);
                }
            }
            GtkChildMountRoute::ExpanderRowRows => {
                for child in previous {
                    child.unparent();
                }
                let expander = parent_widget
                    .clone()
                    .downcast::<adw::ExpanderRow>()
                    .expect("expander row widget should downcast");
                for child in next {
                    expander.add_row(child);
                }
            }
            GtkChildMountRoute::ListBoxChildren => {
                let list_box = parent_widget
                    .clone()
                    .downcast::<gtk::ListBox>()
                    .expect("list box widget should downcast");
                for child in previous {
                    list_box.remove(child);
                }
                for child in next {
                    list_box.append(child);
                }
            }
            GtkChildMountRoute::ListViewChildren => {
                let list_view = parent_widget
                    .clone()
                    .downcast::<gtk::ListView>()
                    .expect("list view widget should downcast");
                let store = selection_model_store(list_view.model())
                    .expect("list view should keep a list store-backed selection model");
                replace_collection_store_children(&store, next);
            }
            GtkChildMountRoute::GridViewChildren => {
                let grid_view = parent_widget
                    .clone()
                    .downcast::<gtk::GridView>()
                    .expect("grid view widget should downcast");
                let store = selection_model_store(grid_view.model())
                    .expect("grid view should keep a list store-backed selection model");
                replace_collection_store_children(&store, next);
            }
            GtkChildMountRoute::NavigationViewPages => {
                for child in previous {
                    child.unparent();
                }
                let nav_view = parent_widget
                    .clone()
                    .downcast::<adw::NavigationView>()
                    .expect("navigation view widget should downcast");
                for child in next {
                    if let Ok(page) = child.clone().downcast::<adw::NavigationPage>() {
                        nav_view.add(&page);
                    }
                }
            }
            GtkChildMountRoute::PreferencesGroupChildren => {
                for child in previous {
                    child.unparent();
                }
                let group = parent_widget
                    .clone()
                    .downcast::<adw::PreferencesGroup>()
                    .expect("preferences group widget should downcast");
                for child in next {
                    group.add(child);
                }
            }
            GtkChildMountRoute::PreferencesPageChildren => {
                for child in previous {
                    child.unparent();
                }
                let page = parent_widget
                    .clone()
                    .downcast::<adw::PreferencesPage>()
                    .expect("preferences page widget should downcast");
                for child in next {
                    if let Ok(group) = child.clone().downcast::<adw::PreferencesGroup>() {
                        page.add(&group);
                    } else {
                        child.unparent();
                    }
                }
            }
            GtkChildMountRoute::PreferencesWindowPages => {
                for child in previous {
                    child.unparent();
                }
                let win = parent_widget
                    .clone()
                    .downcast::<adw::PreferencesWindow>()
                    .expect("preferences window widget should downcast");
                for child in next {
                    if let Ok(page) = child.clone().downcast::<adw::PreferencesPage>() {
                        win.add(&page);
                    } else {
                        child.unparent();
                    }
                }
            }
            GtkChildMountRoute::OverlayOverlay => {
                let overlay = parent_widget
                    .clone()
                    .downcast::<gtk::Overlay>()
                    .expect("overlay widget should downcast");
                for child in previous {
                    overlay.remove_overlay(child);
                }
                for child in next {
                    overlay.add_overlay(child);
                }
            }
            GtkChildMountRoute::ViewStackPages => {
                for child in previous {
                    child.unparent();
                }
                let stack = parent_widget
                    .clone()
                    .downcast::<adw::ViewStack>()
                    .expect("view stack widget should downcast");
                let meta_map = self.view_stack_page_meta.borrow();
                for child in next {
                    let key = child.as_ptr() as usize;
                    let meta = meta_map.get(&key).cloned().unwrap_or_default();
                    let name: Option<&str> = if meta.name.is_empty() {
                        None
                    } else {
                        Some(&meta.name)
                    };
                    stack.add_titled_with_icon(child, name, &meta.title, &meta.icon_name);
                }
            }
            GtkChildMountRoute::FlowBoxChildren => {
                let flow_box = parent_widget
                    .clone()
                    .downcast::<gtk::FlowBox>()
                    .expect("flow box widget should downcast");
                for child in previous {
                    flow_box.remove(child);
                }
                for child in next {
                    flow_box.append(child);
                }
            }
            GtkChildMountRoute::CarouselPages => {
                let carousel = parent_widget
                    .clone()
                    .downcast::<adw::Carousel>()
                    .expect("carousel widget should downcast");
                for child in previous {
                    carousel.remove(child);
                }
                for child in next {
                    carousel.append(child);
                }
            }
            GtkChildMountRoute::TabViewPages => {
                let tab_view = parent_widget
                    .clone()
                    .downcast::<adw::TabView>()
                    .expect("tab view widget should downcast");
                for child in previous {
                    let page = tab_view.page(child);
                    tab_view.close_page(&page);
                    tab_view.close_page_finish(&page, true);
                }
                let meta_map = self.tab_page_meta.borrow();
                for child in next {
                    let page = tab_view.append(child);
                    let key = child.as_ptr() as usize;
                    if let Some(meta) = meta_map.get(&key) {
                        page.set_title(&meta.title);
                        page.set_needs_attention(meta.needs_attention);
                        page.set_loading(meta.loading);
                    }
                }
            }
            GtkChildMountRoute::GridChildren => {
                let grid = parent_widget
                    .clone()
                    .downcast::<gtk::Grid>()
                    .expect("grid widget should downcast");
                for child in previous {
                    grid.remove(child);
                }
                let meta_map = self.grid_child_meta.borrow();
                for child in next {
                    let key = child.as_ptr() as usize;
                    let meta = meta_map.get(&key).cloned().unwrap_or_default();
                    grid.attach(
                        child,
                        meta.column,
                        meta.row,
                        meta.column_span.max(1),
                        meta.row_span.max(1),
                    );
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
            GtkChildMountRoute::WindowTitlebar => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Window>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Window".into(),
                        expected_type: "gtk::Window",
                    })?
                    .set_titlebar(child);
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
            GtkChildMountRoute::StatusPageContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::StatusPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "StatusPage".into(),
                        expected_type: "adw::StatusPage",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ClampContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::Clamp>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Clamp".into(),
                        expected_type: "adw::Clamp",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ToolbarViewContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::ToolbarView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ToolbarView".into(),
                        expected_type: "adw::ToolbarView",
                    })?
                    .set_content(child);
            }
            GtkChildMountRoute::ListBoxRowChild => {
                parent_widget
                    .clone()
                    .downcast::<gtk::ListBoxRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ListBoxRow".into(),
                        expected_type: "gtk::ListBoxRow",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ExpanderChild => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Expander>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Expander".into(),
                        expected_type: "gtk::Expander",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::NavigationPageContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::NavigationPage>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "NavigationPage".into(),
                        expected_type: "adw::NavigationPage",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ToastOverlayContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::ToastOverlay>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ToastOverlay".into(),
                        expected_type: "adw::ToastOverlay",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::OverlayContent => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Overlay>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Overlay".into(),
                        expected_type: "gtk::Overlay",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ViewStackPageContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::Bin>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ViewStackPage".into(),
                        expected_type: "adw::Bin",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::FlowBoxChildContent => {
                parent_widget
                    .clone()
                    .downcast::<gtk::FlowBoxChild>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "FlowBoxChild".into(),
                        expected_type: "gtk::FlowBoxChild",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ButtonChild => {
                let btn = parent_widget
                    .clone()
                    .downcast::<gtk::Button>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Button".into(),
                        expected_type: "gtk::Button",
                    })?;
                match child {
                    Some(c) => btn.set_child(Some(c.upcast_ref::<gtk::Widget>())),
                    None => btn.set_child(None::<&gtk::Widget>),
                }
            }
            GtkChildMountRoute::MenuButtonPopover => {
                let btn = parent_widget
                    .clone()
                    .downcast::<gtk::MenuButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "MenuButton".into(),
                        expected_type: "gtk::MenuButton",
                    })?;
                match child {
                    Some(c) => {
                        if let Ok(popover) = c.clone().downcast::<gtk::Popover>() {
                            btn.set_popover(Some(&popover));
                        }
                    }
                    None => btn.set_popover(None::<&gtk::Popover>),
                }
            }
            GtkChildMountRoute::PopoverContent => {
                parent_widget
                    .clone()
                    .downcast::<gtk::Popover>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "Popover".into(),
                        expected_type: "gtk::Popover",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::CenterBoxStart => {
                parent_widget
                    .clone()
                    .downcast::<gtk::CenterBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "CenterBox".into(),
                        expected_type: "gtk::CenterBox",
                    })?
                    .set_start_widget(child);
            }
            GtkChildMountRoute::CenterBoxCenter => {
                parent_widget
                    .clone()
                    .downcast::<gtk::CenterBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "CenterBox".into(),
                        expected_type: "gtk::CenterBox",
                    })?
                    .set_center_widget(child);
            }
            GtkChildMountRoute::CenterBoxEnd => {
                parent_widget
                    .clone()
                    .downcast::<gtk::CenterBox>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "CenterBox".into(),
                        expected_type: "gtk::CenterBox",
                    })?
                    .set_end_widget(child);
            }
            GtkChildMountRoute::NavigationSplitViewSidebar => {
                let nav = parent_widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "NavigationSplitView".into(),
                        expected_type: "adw::NavigationSplitView",
                    })?;
                match child {
                    Some(c) => {
                        if let Ok(page) = c.clone().downcast::<adw::NavigationPage>() {
                            nav.set_sidebar(Some(&page));
                        }
                    }
                    None => nav.set_sidebar(None::<&adw::NavigationPage>),
                }
            }
            GtkChildMountRoute::NavigationSplitViewContent => {
                let nav = parent_widget
                    .clone()
                    .downcast::<adw::NavigationSplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "NavigationSplitView".into(),
                        expected_type: "adw::NavigationSplitView",
                    })?;
                match child {
                    Some(c) => {
                        if let Ok(page) = c.clone().downcast::<adw::NavigationPage>() {
                            nav.set_content(Some(&page));
                        }
                    }
                    None => nav.set_content(None::<&adw::NavigationPage>),
                }
            }
            GtkChildMountRoute::OverlaySplitViewSidebar => {
                parent_widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "OverlaySplitView".into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_sidebar(child);
            }
            GtkChildMountRoute::OverlaySplitViewContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::OverlaySplitView>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "OverlaySplitView".into(),
                        expected_type: "adw::OverlaySplitView",
                    })?
                    .set_content(child);
            }
            GtkChildMountRoute::TabViewTabBar => {
                if let Some(tab_view) = parent_widget.clone().downcast::<adw::TabView>().ok() {
                    if let Some(c) = child {
                        if let Ok(bar) = c.clone().downcast::<adw::TabBar>() {
                            bar.set_view(Some(&tab_view));
                        }
                    }
                }
            }
            GtkChildMountRoute::TabPageContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::Bin>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "TabPage".into(),
                        expected_type: "adw::Bin",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::GridChildContent => {
                parent_widget
                    .clone()
                    .downcast::<adw::Bin>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "GridChild".into(),
                        expected_type: "adw::Bin",
                    })?
                    .set_child(child);
            }
            GtkChildMountRoute::ActionRowPrefix => {
                let row = parent_widget
                    .clone()
                    .downcast::<adw::ActionRow>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "ActionRow".into(),
                        expected_type: "adw::ActionRow",
                    })?;
                if let Some(c) = child {
                    row.add_prefix(c);
                }
            }
            GtkChildMountRoute::SplitButtonPopover => {
                let btn = parent_widget
                    .clone()
                    .downcast::<adw::SplitButton>()
                    .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                        widget: "SplitButton".into(),
                        expected_type: "adw::SplitButton",
                    })?;
                match child {
                    Some(c) => {
                        if let Ok(popover) = c.clone().downcast::<gtk::Popover>() {
                            btn.set_popover(Some(&popover));
                        }
                    }
                    None => btn.set_popover(None::<&gtk::Popover>),
                }
            }
            GtkChildMountRoute::CarouselDots => {
                // CarouselIndicatorDots connects to the carousel via its set_carousel call
                // when mounted; the dots widget has no relationship to the parent Carousel here.
                let _ = child;
            }
            GtkChildMountRoute::CarouselLines => {
                let _ = child;
            }
            GtkChildMountRoute::HeaderBarStart
            | GtkChildMountRoute::HeaderBarEnd
            | GtkChildMountRoute::BoxChildren
            | GtkChildMountRoute::ToolbarViewTop
            | GtkChildMountRoute::ToolbarViewBottom
            | GtkChildMountRoute::ActionRowSuffix
            | GtkChildMountRoute::ExpanderRowRows
            | GtkChildMountRoute::ListBoxChildren
            | GtkChildMountRoute::ListViewChildren
            | GtkChildMountRoute::GridViewChildren
            | GtkChildMountRoute::NavigationViewPages
            | GtkChildMountRoute::PreferencesGroupChildren
            | GtkChildMountRoute::PreferencesPageChildren
            | GtkChildMountRoute::PreferencesWindowPages
            | GtkChildMountRoute::OverlayOverlay
            | GtkChildMountRoute::ViewStackPages
            | GtkChildMountRoute::FlowBoxChildren
            | GtkChildMountRoute::CarouselPages
            | GtkChildMountRoute::TabViewPages
            | GtkChildMountRoute::GridChildren => {
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
        if schema.is_window_root() {
            self.setup_dark_mode_watcher();
            self.setup_clipboard_watcher();
            if let Ok(window) = widget.clone().downcast::<gtk::Window>() {
                self.setup_window_size_watcher(&window);
                self.setup_window_focus_watcher(&window);
            }
        }
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
        let mut signal_object: glib::Object = widget.clone().upcast();
        let event = schema.event(route.binding.name.text()).ok_or_else(|| {
            GtkConcreteHostError::UnsupportedEvent {
                widget: schema.markup_name.into(),
                event: route.binding.name.text().to_owned().into_boxed_str(),
            }
        })?;
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
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
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SpinButtonValueChanged => widget
                .clone()
                .downcast::<gtk::SpinButton>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::SpinButton",
                })?
                .connect_value_changed(move |spin| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_f64(spin.value()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ScaleValueChanged => widget
                .clone()
                .downcast::<gtk::Scale>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Scale",
                })?
                .connect_value_changed(move |scale| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_f64(scale.value()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::RevealerChildRevealed => widget
                .clone()
                .downcast::<gtk::Revealer>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Revealer",
                })?
                .connect_child_revealed_notify(move |r| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(r.is_child_revealed()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::FocusIn => {
                let controller = gtk::EventControllerFocus::new();
                let sid = controller.connect_enter(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::FocusOut => {
                let controller = gtk::EventControllerFocus::new();
                let sid = controller.connect_leave(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::Scroll => {
                let controller =
                    gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
                let sid = controller.connect_scroll(move |_, _, dy| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_f64(dy),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                    glib::Propagation::Proceed
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::PointerEnter => {
                let controller = gtk::EventControllerMotion::new();
                let sid = controller.connect_enter(move |_, _, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::PointerLeave => {
                let controller = gtk::EventControllerMotion::new();
                let sid = controller.connect_leave(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::BannerButtonClicked => widget
                .clone()
                .downcast::<adw::Banner>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::Banner",
                })?
                .connect_button_clicked(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ActionRowActivated => widget
                .clone()
                .downcast::<adw::ActionRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::ActionRow",
                })?
                .connect_activated(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SwitchRowToggled => widget
                .clone()
                .downcast::<adw::SwitchRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::SwitchRow",
                })?
                .connect_active_notify(move |row| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(row.is_active()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SpinRowValueChanged => widget
                .clone()
                .downcast::<adw::SpinRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::SpinRow",
                })?
                .connect_value_notify(move |row| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_f64(row.value()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::EntryRowChanged => widget
                .clone()
                .downcast::<adw::EntryRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::EntryRow",
                })?
                .connect_changed(move |entry| {
                    let text = entry.text();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::EntryRowActivated => widget
                .clone()
                .downcast::<adw::EntryRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::EntryRow",
                })?
                .connect_entry_activated(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ListBoxActivated => widget
                .clone()
                .downcast::<gtk::ListBox>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::ListBox",
                })?
                .connect_row_activated(move |_, row| {
                    let index = row.index() as i64;
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(index),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ListBoxRowActivated => widget
                .clone()
                .downcast::<gtk::ListBoxRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::ListBoxRow",
                })?
                .connect_activate(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ListViewActivated => widget
                .clone()
                .downcast::<gtk::ListView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::ListView",
                })?
                .connect_activate(move |_, position| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(position as i64),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::GridViewActivated => widget
                .clone()
                .downcast::<gtk::GridView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::GridView",
                })?
                .connect_activate(move |_, position| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(position as i64),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::DropDownSelectionChanged => widget
                .clone()
                .downcast::<gtk::DropDown>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::DropDown",
                })?
                .connect_selected_notify(move |dd| {
                    let selected = dd.selected() as i64;
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(selected),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SearchEntryChanged => widget
                .clone()
                .downcast::<gtk::SearchEntry>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::SearchEntry",
                })?
                .connect_changed(move |entry| {
                    let text = entry.text();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SearchEntryActivated => widget
                .clone()
                .downcast::<gtk::SearchEntry>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::SearchEntry",
                })?
                .connect_activate(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SearchEntrySearchChanged => widget
                .clone()
                .downcast::<gtk::SearchEntry>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::SearchEntry",
                })?
                .connect_search_changed(move |entry| {
                    let text = entry.text();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ComboRowSelectionChanged => widget
                .clone()
                .downcast::<adw::ComboRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::ComboRow",
                })?
                .connect_selected_notify(move |row| {
                    let selected = row.selected() as i64;
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(selected),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::PasswordEntryRowChanged => widget
                .clone()
                .downcast::<adw::PasswordEntryRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::PasswordEntryRow",
                })?
                .connect_changed(move |entry| {
                    let text = entry.text();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::PasswordEntryRowActivated => widget
                .clone()
                .downcast::<adw::PasswordEntryRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::PasswordEntryRow",
                })?
                .connect_entry_activated(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::MultilineEntryChanged => {
                let text_view = widget.clone().downcast::<gtk::TextView>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::TextView",
                    }
                })?;
                let buffer = text_view.buffer();
                let sid = buffer.connect_changed(move |buf| {
                    let (start, end) = buf.bounds();
                    let text = buf.text(&start, &end, false);
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(text.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = buffer.upcast::<glib::Object>();
                sid
            }
            GtkEventSignal::WindowCloseRequest => widget
                .clone()
                .downcast::<gtk::Window>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Window",
                })?
                .connect_close_request(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                    glib::Propagation::Proceed
                }),
            GtkEventSignal::NavigationViewPopped => widget
                .clone()
                .downcast::<adw::NavigationView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::NavigationView",
                })?
                .connect_popped(move |_, page| {
                    let tag = page.tag().unwrap_or_default();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(tag.as_str()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ExpanderRowExpanded => widget
                .clone()
                .downcast::<adw::ExpanderRow>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::ExpanderRow",
                })?
                .connect_expanded_notify(move |row| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(row.is_expanded()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ExpanderExpanded => widget
                .clone()
                .downcast::<gtk::Expander>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Expander",
                })?
                .connect_expanded_notify(move |expander| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(expander.is_expanded()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::NavigationPageShowing => widget
                .clone()
                .downcast::<adw::NavigationPage>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::NavigationPage",
                })?
                .connect_showing(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::NavigationPageHiding => widget
                .clone()
                .downcast::<adw::NavigationPage>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::NavigationPage",
                })?
                .connect_hiding(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::ViewStackSwitch => widget
                .clone()
                .downcast::<adw::ViewStack>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::ViewStack",
                })?
                .connect_visible_child_name_notify(move |stack| {
                    let name = stack
                        .visible_child_name()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(&name),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::AlertDialogResponse => widget
                .clone()
                .downcast::<adw::MessageDialog>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::MessageDialog",
                })?
                .connect_response(None, move |_, response| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_text(response),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::WindowMaximized => widget
                .clone()
                .downcast::<gtk::Window>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Window",
                })?
                .connect_maximized_notify(move |win| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(win.is_maximized()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::WindowFullscreened => widget
                .clone()
                .downcast::<gtk::Window>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Window",
                })?
                .connect_fullscreened_notify(move |win| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(win.is_fullscreen()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::CalendarDaySelected => widget
                .clone()
                .downcast::<gtk::Calendar>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Calendar",
                })?
                .connect_day_selected(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::FlowBoxChildActivated => widget
                .clone()
                .downcast::<gtk::FlowBox>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::FlowBox",
                })?
                .connect_child_activated(move |_, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::MenuButtonToggled => {
                let btn = widget.clone().downcast::<gtk::MenuButton>().map_err(|_| {
                    GtkConcreteHostError::WidgetDowncastFailed {
                        widget: schema.markup_name.into(),
                        expected_type: "gtk::MenuButton",
                    }
                })?;
                use glib::prelude::ObjectExt as _;
                btn.connect_notify_local(Some("active"), move |btn, _| {
                    use glib::prelude::ObjectExt as _;
                    let active: bool = btn.property("active");
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(active),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                })
            }
            GtkEventSignal::PopoverClosed => widget
                .clone()
                .downcast::<gtk::Popover>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "gtk::Popover",
                })?
                .connect_closed(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::SecondaryClick => {
                let controller = gtk::GestureClick::new();
                controller.set_button(3);
                let sid = controller.connect_pressed(move |_, _, _, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::LongPress => {
                let controller = gtk::GestureLongPress::new();
                let sid = controller.connect_pressed(move |_, _, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::SwipeLeft => {
                let controller = gtk::GestureSwipe::new();
                let sid = controller.connect_swipe(move |_, velocity_x, _| {
                    if velocity_x < 0.0 {
                        queue.push(GtkQueuedEvent {
                            route: route_id,
                            value: V::unit(),
                        });
                        if let Some(notifier) = notifier.borrow().clone() {
                            notifier();
                        }
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::SwipeRight => {
                let controller = gtk::GestureSwipe::new();
                let sid = controller.connect_swipe(move |_, velocity_x, _| {
                    if velocity_x > 0.0 {
                        queue.push(GtkQueuedEvent {
                            route: route_id,
                            value: V::unit(),
                        });
                        if let Some(notifier) = notifier.borrow().clone() {
                            notifier();
                        }
                    }
                });
                signal_object = controller.clone().upcast::<glib::Object>();
                widget.add_controller(controller);
                sid
            }
            GtkEventSignal::NavigationSplitViewShowContentChanged => widget
                .clone()
                .downcast::<adw::NavigationSplitView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::NavigationSplitView",
                })?
                .connect_show_content_notify(move |nav| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(nav.shows_content()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::OverlaySplitViewShowSidebarChanged => widget
                .clone()
                .downcast::<adw::OverlaySplitView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::OverlaySplitView",
                })?
                .connect_show_sidebar_notify(move |ov| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_bool(ov.shows_sidebar()),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::TabViewPageAdded => widget
                .clone()
                .downcast::<adw::TabView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::TabView",
                })?
                .connect_page_attached(move |_, _, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::TabViewPageClosed => widget
                .clone()
                .downcast::<adw::TabView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::TabView",
                })?
                .connect_close_page(move |_, _| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                    glib::Propagation::Proceed
                }),
            GtkEventSignal::TabViewSelectedPageChanged => widget
                .clone()
                .downcast::<adw::TabView>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::TabView",
                })?
                .connect_selected_page_notify(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::CarouselPageChanged => widget
                .clone()
                .downcast::<adw::Carousel>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::Carousel",
                })?
                .connect_page_changed(move |_, idx| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::from_i64(idx as i64),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
            GtkEventSignal::FileDialogResponse => {
                let key = widget.as_ptr() as usize;
                if let Some(state) = self.file_chooser_states.borrow().get(&key) {
                    let native = state.native.clone();
                    native.connect_response(move |_, response| {
                        let val = match response {
                            gtk::ResponseType::Accept => 1,
                            gtk::ResponseType::Cancel => 0,
                            gtk::ResponseType::DeleteEvent => -4,
                            gtk::ResponseType::Other(v) => v as i64,
                            _ => -1,
                        };
                        queue.push(GtkQueuedEvent {
                            route: route_id,
                            value: V::from_i64(val),
                        });
                        if let Some(notifier) = notifier.borrow().clone() {
                            notifier();
                        }
                    })
                } else {
                    widget.connect_notify_local(Some("visible"), move |_, _| {
                        queue.push(GtkQueuedEvent {
                            route: route_id,
                            value: V::from_i64(0),
                        });
                        if let Some(notifier) = notifier.borrow().clone() {
                            notifier();
                        }
                    })
                }
            }
            GtkEventSignal::SplitButtonClicked => widget
                .clone()
                .downcast::<adw::SplitButton>()
                .map_err(|_| GtkConcreteHostError::WidgetDowncastFailed {
                    widget: schema.markup_name.into(),
                    expected_type: "adw::SplitButton",
                })?
                .connect_clicked(move |_| {
                    queue.push(GtkQueuedEvent {
                        route: route_id,
                        value: V::unit(),
                    });
                    if let Some(notifier) = notifier.borrow().clone() {
                        notifier();
                    }
                }),
        };
        self.events.insert(
            handle.0,
            MountedEvent {
                widget: widget.clone(),
                signal_object,
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
        mounted.signal_object.disconnect(mounted.signal);
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
                event.signal_object.disconnect(event.signal);
            }
        }
        self.managed_css_classes
            .borrow_mut()
            .remove(&(mounted.widget.as_ptr() as usize));
        self.alert_dialog_responses
            .borrow_mut()
            .remove(&(mounted.widget.as_ptr() as usize));
        self.view_stack_page_meta
            .borrow_mut()
            .remove(&(mounted.widget.as_ptr() as usize));
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
    signal_object: glib::Object,
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

fn parse_align(value: &str) -> Option<gtk::Align> {
    match value {
        "Fill" => Some(gtk::Align::Fill),
        "Start" => Some(gtk::Align::Start),
        "End" => Some(gtk::Align::End),
        "Center" => Some(gtk::Align::Center),
        "Baseline" => Some(gtk::Align::Baseline),
        _ => None,
    }
}

fn parse_wrap_mode(value: &str) -> Option<gtk::pango::WrapMode> {
    match value {
        "Word" => Some(gtk::pango::WrapMode::Word),
        "Char" => Some(gtk::pango::WrapMode::Char),
        "WordChar" => Some(gtk::pango::WrapMode::WordChar),
        _ => None,
    }
}

fn parse_justification(value: &str) -> Option<gtk::Justification> {
    match value {
        "Left" => Some(gtk::Justification::Left),
        "Center" => Some(gtk::Justification::Center),
        "Right" => Some(gtk::Justification::Right),
        "Fill" => Some(gtk::Justification::Fill),
        _ => None,
    }
}

fn parse_ellipsize(value: &str) -> Option<gtk::pango::EllipsizeMode> {
    match value {
        "None" => Some(gtk::pango::EllipsizeMode::None),
        "Start" => Some(gtk::pango::EllipsizeMode::Start),
        "Middle" => Some(gtk::pango::EllipsizeMode::Middle),
        "End" => Some(gtk::pango::EllipsizeMode::End),
        _ => None,
    }
}

fn parse_input_purpose(value: &str) -> Option<gtk::InputPurpose> {
    match value {
        "FreeForm" => Some(gtk::InputPurpose::FreeForm),
        "Alpha" => Some(gtk::InputPurpose::Alpha),
        "Digits" => Some(gtk::InputPurpose::Digits),
        "Number" => Some(gtk::InputPurpose::Number),
        "Phone" => Some(gtk::InputPurpose::Phone),
        "Url" => Some(gtk::InputPurpose::Url),
        "Email" => Some(gtk::InputPurpose::Email),
        "Name" => Some(gtk::InputPurpose::Name),
        "Password" => Some(gtk::InputPurpose::Password),
        "Pin" => Some(gtk::InputPurpose::Pin),
        _ => None,
    }
}

fn parse_policy(value: &str) -> Option<gtk::PolicyType> {
    match value {
        "Always" => Some(gtk::PolicyType::Always),
        "Automatic" => Some(gtk::PolicyType::Automatic),
        "Never" => Some(gtk::PolicyType::Never),
        "External" => Some(gtk::PolicyType::External),
        _ => None,
    }
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

    #[derive(Clone, Debug, PartialEq)]
    enum TestValue {
        Bool(bool),
        Int(i64),
        Float(f64),
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

        fn from_f64(v: f64) -> Self {
            Self::Float(v)
        }

        fn from_i64(v: i64) -> Self {
            Self::Int(v)
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

        fn as_f64(&self) -> Option<f64> {
            match self {
                Self::Float(value) => Some(*value),
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

    #[test]
    fn window_size_queue_dedupes_repeated_values() {
        let queue = GtkWindowSizeQueue::default();

        assert!(queue.push((1280, 720)));
        assert!(!queue.push((1280, 720)));

        assert_eq!(queue.drain(), vec![(1280, 720)]);

        assert!(!queue.push((1280, 720)));
        assert!(queue.drain().is_empty());

        assert!(queue.push((1440, 860)));
        assert_eq!(queue.drain(), vec![(1440, 860)]);
    }

    #[test]
    fn window_focus_queue_dedupes_repeated_values() {
        let queue = GtkWindowFocusQueue::default();

        assert!(queue.push(true));
        assert!(!queue.push(true));

        assert_eq!(queue.drain(), vec![true]);

        assert!(!queue.push(true));
        assert!(queue.drain().is_empty());

        assert!(queue.push(false));
        assert_eq!(queue.drain(), vec![false]);
    }

    #[test]
    fn window_size_snapshot_notifier_skips_duplicate_values() {
        let queue = GtkWindowSizeQueue::default();
        let notify_count = Rc::new(std::cell::Cell::new(0usize));
        let notify_count_for_closure = notify_count.clone();
        let notifier: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
            Rc::new(RefCell::new(Some(Rc::new(move || {
                notify_count_for_closure.set(notify_count_for_closure.get() + 1);
            }))));

        queue_window_size_snapshot(&queue, &notifier, (1280, 720));
        queue_window_size_snapshot(&queue, &notifier, (1280, 720));
        assert_eq!(notify_count.get(), 1);
        assert_eq!(queue.drain(), vec![(1280, 720)]);

        queue_window_size_snapshot(&queue, &notifier, (1280, 720));
        assert_eq!(notify_count.get(), 1);
        assert!(queue.drain().is_empty());

        queue_window_size_snapshot(&queue, &notifier, (1440, 860));
        assert_eq!(notify_count.get(), 2);
        assert_eq!(queue.drain(), vec![(1440, 860)]);
    }

    #[test]
    fn window_focus_snapshot_notifier_skips_duplicate_values() {
        let queue = GtkWindowFocusQueue::default();
        let notify_count = Rc::new(std::cell::Cell::new(0usize));
        let notify_count_for_closure = notify_count.clone();
        let notifier: Rc<RefCell<Option<Rc<dyn Fn()>>>> =
            Rc::new(RefCell::new(Some(Rc::new(move || {
                notify_count_for_closure.set(notify_count_for_closure.get() + 1);
            }))));

        queue_window_focus_snapshot(&queue, &notifier, true);
        queue_window_focus_snapshot(&queue, &notifier, true);
        assert_eq!(notify_count.get(), 1);
        assert_eq!(queue.drain(), vec![true]);

        queue_window_focus_snapshot(&queue, &notifier, true);
        assert_eq!(notify_count.get(), 1);
        assert!(queue.drain().is_empty());

        queue_window_focus_snapshot(&queue, &notifier, false);
        assert_eq!(notify_count.get(), 2);
        assert_eq!(queue.drain(), vec![false]);
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

    fn collection_store_widgets(model: Option<gtk::SelectionModel>) -> Vec<gtk::Widget> {
        let store = selection_model_store(model)
            .expect("collection views should keep a list store-backed selection model");
        (0..store.n_items())
            .map(|index| {
                let boxed = store
                    .item(index)
                    .expect("list store item should exist")
                    .downcast::<glib::BoxedAnyObject>()
                    .expect("list store items should be boxed widgets");
                boxed.borrow::<gtk::Widget>().clone()
            })
            .collect()
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
value titleText = "Runtime title"
value gap = 4
value isVisible = False
value isEnabled = True
value click = True
value view =
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
    fn concrete_host_mounts_action_row_prefix_sequences() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-action-row-prefix.aivi",
                r#"
value view =
    <Window title="Host">
        <ListBox>
            <ActionRow title="Thread" subtitle="Preview">
                <ActionRow.prefix>
                    <Label text="Mail" />
                </ActionRow.prefix>
            </ActionRow>
        </ListBox>
    </Window>
"#,
            );
            let executor = GtkRuntimeExecutor::new(graph, GtkConcreteHost::<TestValue>::default())
                .expect("concrete GTK host should mount action row prefix sequences");
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
            let list_box = window
                .child()
                .expect("window should have a child")
                .downcast::<gtk::ListBox>()
                .expect("window child should be a GTK list box");
            let row = list_box
                .first_child()
                .expect("list box should mount an action row")
                .downcast::<adw::ActionRow>()
                .expect("list box row should be an action row");
            assert_eq!(row.title(), "Thread");
        });
    }

    #[test]
    fn concrete_host_notifier_installed_after_mount_wakes_existing_button() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-late-notifier.aivi",
                r#"
value click = True
value view =
    <Window title="Host">
        <Button label="Restart" onClick={click} />
    </Window>
"#,
            );
            let mut executor =
                GtkRuntimeExecutor::new(graph, GtkConcreteHost::<TestValue>::default())
                    .expect("concrete GTK host should mount the button before notifier wiring");

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

            let notify_count = Rc::new(std::cell::Cell::new(0usize));
            let notify_count_for_closure = notify_count.clone();
            executor
                .host_mut()
                .set_event_notifier(Some(Rc::new(move || {
                    notify_count_for_closure.set(notify_count_for_closure.get() + 1);
                })));

            button.emit_clicked();
            assert_eq!(
                notify_count.get(),
                1,
                "buttons mounted before notifier wiring should still notify once the notifier is installed"
            );
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
value query = "Runtime query"
value canEdit = False
value isEnabled = True
value submit = True
value view =
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
value query = "Runtime query"
signal changed : Signal Text

value view =
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
value showButtons = False
value isEnabled = True
value toggled = True
value view =
    <Window title="Host">
        <Viewport>
            <Frame label="Controls">
                <Box>
                    <HeaderBar showTitleButtons={showButtons}>
                        <HeaderBar.titleWidget>
                            <Label text="Profile" />
                        </HeaderBar.titleWidget>
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
value view =
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
    fn concrete_host_mounts_window_titlebars_and_compact_buttons() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-window-titlebar.aivi",
                r#"
value showButtons = True
value view =
    <Window title="Host">
        <Window.titlebar>
            <HeaderBar showTitleButtons={showButtons}>
                <HeaderBar.start>
                    <Label text="Status" />
                </HeaderBar.start>
                <HeaderBar.end>
                    <Button label="Restart" compact hasFrame={False} widthRequest={26} heightRequest={26} />
                </HeaderBar.end>
            </HeaderBar>
        </Window.titlebar>
        <Button label="A" compact />
    </Window>
"#,
            );
            let show_title_buttons_input =
                find_widget_input(&graph, "HeaderBar", "showTitleButtons");
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [(show_title_buttons_input, TestValue::Bool(true))],
            )
            .expect("concrete GTK host should mount a window titlebar");

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
            let titlebar = window
                .titlebar()
                .expect("window should mount a titlebar child")
                .downcast::<gtk::HeaderBar>()
                .expect("titlebar child should be a header bar");
            assert!(titlebar.property::<bool>("show-title-buttons"));

            let content = window
                .child()
                .expect("window should keep the board button as content")
                .downcast::<gtk::Button>()
                .expect("window content should be a button");
            assert!(content.has_css_class("aivi-compact-button"));
        });
    }

    #[test]
    fn concrete_host_applies_compact_borderless_button_properties() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-button-frame.aivi",
                r#"
value framed = False
value width = 26
value height = 26
value view =
    <Window title="Host">
        <Button label="A" compact hasFrame={framed} widthRequest={width} heightRequest={height} />
    </Window>
"#,
            );
            let frame_input = find_widget_input(&graph, "Button", "hasFrame");
            let width_input = find_widget_input(&graph, "Button", "widthRequest");
            let height_input = find_widget_input(&graph, "Button", "heightRequest");
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (frame_input, TestValue::Bool(false)),
                    (width_input, TestValue::Int(26)),
                    (height_input, TestValue::Int(26)),
                ],
            )
            .expect("concrete GTK host should mount a compact borderless button");

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
            let content = window
                .child()
                .expect("window should keep the button as content")
                .downcast::<gtk::Button>()
                .expect("window content should be a button");
            assert!(content.has_css_class("aivi-compact-button"));
            assert!(!content.has_frame());
            assert_eq!(content.width_request(), 26);
            assert_eq!(content.height_request(), 26);
        });
    }

    #[test]
    fn concrete_host_applies_button_focusable_property() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-button-focusable.aivi",
                r#"
value isFocusable = False
value view =
    <Window title="Host">
        <Button label="A" focusable={isFocusable} />
    </Window>
"#,
            );
            let focusable_input = find_widget_input(&graph, "Button", "focusable");
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [(focusable_input, TestValue::Bool(false))],
            )
            .expect("concrete GTK host should mount a button with focusability");

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
            let content = window
                .child()
                .expect("window should keep the button as content")
                .downcast::<gtk::Button>()
                .expect("window content should be a button");
            assert!(!content.is_focusable());
        });
    }

    #[test]
    fn concrete_host_applies_button_opacity_and_transition_class() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-button-opacity.aivi",
                r#"
value shouldAnimate = True
value buttonOpacity = 0.35
value view =
    <Window title="Host">
        <Button label="A" animateOpacity={shouldAnimate} opacity={buttonOpacity} />
    </Window>
"#,
            );
            let animate_input = find_widget_input(&graph, "Button", "animateOpacity");
            let opacity_input = find_widget_input(&graph, "Button", "opacity");
            let executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (animate_input, TestValue::Bool(true)),
                    (opacity_input, TestValue::Float(0.35)),
                ],
            )
            .expect("concrete GTK host should mount an animated opacity button");

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
            let content = window
                .child()
                .expect("window should keep the button as content")
                .downcast::<gtk::Button>()
                .expect("window content should be a button");
            assert!(content.has_css_class("aivi-animate-opacity"));
            assert!((content.property::<f64>("opacity") - 0.35).abs() < 0.001);
        });
    }

    #[test]
    fn concrete_host_mounts_named_child_groups_for_paned_and_header_bar() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-named-groups.aivi",
                r#"
value showButtons = False
value view =
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
value view =
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
value view =
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
value view =
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

    #[test]
    fn concrete_host_mounts_virtual_collection_views_and_routes_activation() {
        gtk::test_synced(|| {
            let graph = lower_graph(
                "gtk-host-virtual-collections.aivi",
                r#"
signal listShowSeparators : Signal Bool
signal listEnableRubberband : Signal Bool
signal listSingleClickActivate : Signal Bool
signal gridEnableRubberband : Signal Bool
signal gridSingleClickActivate : Signal Bool
signal gridMinColumns : Signal Int
signal gridMaxColumns : Signal Int

value view =
    <Window title="Host">
        <Box orientation="Vertical">
            <ListView
                showSeparators={listShowSeparators}
                enableRubberband={listEnableRubberband}
                singleClickActivate={listSingleClickActivate}
                onActivate={.}
            >
                <Label text="Alpha" />
                <Label text="Beta" />
                <Label text="Gamma" />
            </ListView>
            <GridView
                minColumns={gridMinColumns}
                maxColumns={gridMaxColumns}
                enableRubberband={gridEnableRubberband}
                singleClickActivate={gridSingleClickActivate}
                onActivate={.}
            >
                <Button label="One" />
                <Button label="Two" />
            </GridView>
        </Box>
    </Window>
"#,
            );
            let list_show_separators_input =
                find_widget_input(&graph, "ListView", "showSeparators");
            let list_enable_rubberband_input =
                find_widget_input(&graph, "ListView", "enableRubberband");
            let list_single_click_activate_input =
                find_widget_input(&graph, "ListView", "singleClickActivate");
            let grid_enable_rubberband_input =
                find_widget_input(&graph, "GridView", "enableRubberband");
            let grid_single_click_activate_input =
                find_widget_input(&graph, "GridView", "singleClickActivate");
            let grid_min_columns_input = find_widget_input(&graph, "GridView", "minColumns");
            let grid_max_columns_input = find_widget_input(&graph, "GridView", "maxColumns");
            let mut executor = GtkRuntimeExecutor::new_with_values(
                graph,
                GtkConcreteHost::<TestValue>::default(),
                [
                    (list_show_separators_input, TestValue::Bool(true)),
                    (list_enable_rubberband_input, TestValue::Bool(true)),
                    (list_single_click_activate_input, TestValue::Bool(true)),
                    (grid_enable_rubberband_input, TestValue::Bool(true)),
                    (grid_single_click_activate_input, TestValue::Bool(true)),
                    (grid_min_columns_input, TestValue::Int(2)),
                    (grid_max_columns_input, TestValue::Int(4)),
                ],
            )
            .expect("concrete GTK host should mount virtual collection widgets");

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
            let child_handles = executor
                .host()
                .child_handles(&container_handle)
                .expect("box child order should be tracked");
            assert_eq!(child_handles.len(), 2);

            let list_handle = child_handles[0].clone();
            let grid_handle = child_handles[1].clone();

            let list_view = executor
                .host()
                .widget(&list_handle)
                .expect("list view handle should resolve")
                .downcast::<gtk::ListView>()
                .expect("first child should be a GTK list view");
            let grid_view = executor
                .host()
                .widget(&grid_handle)
                .expect("grid view handle should resolve")
                .downcast::<gtk::GridView>()
                .expect("second child should be a GTK grid view");

            assert!(list_view.shows_separators());
            assert!(list_view.enables_rubberband());
            assert!(list_view.is_single_click_activate());
            assert_eq!(grid_view.min_columns(), 2);
            assert_eq!(grid_view.max_columns(), 4);
            assert!(grid_view.enables_rubberband());
            assert!(grid_view.is_single_click_activate());

            let list_labels = collection_store_widgets(list_view.model())
                .into_iter()
                .map(|widget| {
                    widget
                        .downcast::<gtk::Label>()
                        .expect("list view model items should stay labels")
                        .text()
                        .to_string()
                })
                .collect::<Vec<_>>();
            assert_eq!(list_labels, vec!["Alpha", "Beta", "Gamma"]);

            let grid_labels = collection_store_widgets(grid_view.model())
                .into_iter()
                .map(|widget| {
                    widget
                        .downcast::<gtk::Button>()
                        .expect("grid view model items should stay buttons")
                        .label()
                        .expect("grid buttons should have labels")
                        .to_string()
                })
                .collect::<Vec<_>>();
            assert_eq!(grid_labels, vec!["One", "Two"]);

            let before = executor
                .host()
                .child_handles(&list_handle)
                .expect("list view child order should be tracked");
            executor
                .host_mut()
                .move_children(
                    &list_handle,
                    crate::lookup_widget_schema_by_name("ListView")
                        .and_then(|schema| schema.child_group("children"))
                        .expect("ListView should expose its default children group"),
                    0,
                    1,
                    2,
                    &[before[0].clone()],
                )
                .expect("moving list view children should update the backing model");

            let moved_list_labels = collection_store_widgets(list_view.model())
                .into_iter()
                .map(|widget| {
                    widget
                        .downcast::<gtk::Label>()
                        .expect("moved list items should stay labels")
                        .text()
                        .to_string()
                })
                .collect::<Vec<_>>();
            assert_eq!(moved_list_labels, vec!["Beta", "Gamma", "Alpha"]);

            let routes = executor.event_routes();
            assert_eq!(routes.len(), 2);
            list_view.emit_by_name::<()>("activate", &[&1u32]);
            grid_view.emit_by_name::<()>("activate", &[&0u32]);
            let queued = executor.host_mut().drain_events();
            assert_eq!(queued.len(), 2);
            assert_eq!(queued[0].route, routes[0].id);
            assert_eq!(queued[0].value, TestValue::Int(1));
            assert_eq!(queued[1].route, routes[1].id);
            assert_eq!(queued[1].value, TestValue::Int(0));
        });
    }
}
