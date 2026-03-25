use aivi_hir::NamePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteEventPayload {
    Unit,
    Bool,
}

impl GtkConcreteEventPayload {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unit => "Unit",
            Self::Bool => "Bool",
        }
    }

    pub const fn required_signal_type_label(self) -> &'static str {
        match self {
            Self::Unit => "`Signal Unit`",
            Self::Bool => "`Signal Bool`",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteWidgetKind {
    Window,
    Box,
    ScrolledWindow,
    Label,
    Button,
    Entry,
    Switch,
    CheckButton,
    ToggleButton,
    Image,
    Spinner,
    ProgressBar,
    Revealer,
}

impl GtkConcreteWidgetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::Box => "Box",
            Self::ScrolledWindow => "ScrolledWindow",
            Self::Label => "Label",
            Self::Button => "Button",
            Self::Entry => "Entry",
            Self::Switch => "Switch",
            Self::CheckButton => "CheckButton",
            Self::ToggleButton => "ToggleButton",
            Self::Image => "Image",
            Self::Spinner => "Spinner",
            Self::ProgressBar => "ProgressBar",
            Self::Revealer => "Revealer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkWidgetRootKind {
    Window,
    Embedded,
}

impl GtkWidgetRootKind {
    pub const fn is_window(self) -> bool {
        matches!(self, Self::Window)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GtkEnumValueShape {
    pub name: &'static str,
    pub variants: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkPropertyValueShape {
    Bool,
    Text,
    I64,
    F64,
    Enum(GtkEnumValueShape),
}

impl GtkPropertyValueShape {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Text => "Text",
            Self::I64 => "Int",
            Self::F64 => "Float",
            Self::Enum(shape) => shape.name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkBoolPropertySetter {
    Visible,
    Sensitive,
    Hexpand,
    Vexpand,
    EntryEditable,
    SwitchActive,
    CheckButtonActive,
    ToggleButtonActive,
    SpinnerSpinning,
    RevealerRevealed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkTextPropertySetter {
    WindowTitle,
    LabelText,
    LabelLabel,
    ButtonLabel,
    BoxOrientation,
    EntryText,
    EntryPlaceholderText,
    CheckButtonLabel,
    ToggleButtonLabel,
    ImageIconName,
    ImageResourcePath,
    ProgressBarText,
    RevealerTransitionType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkTextOrI64PropertySetter {
    BoxSpacing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkI64PropertySetter {
    ImagePixelSize,
    RevealerTransitionDuration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkF64PropertySetter {
    ProgressBarFraction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkPropertySetter {
    Bool(GtkBoolPropertySetter),
    Text(GtkTextPropertySetter),
    TextOrI64(GtkTextOrI64PropertySetter),
    I64(GtkI64PropertySetter),
    F64(GtkF64PropertySetter),
}

impl GtkPropertySetter {
    pub const fn host_value_label(self) -> &'static str {
        match self {
            Self::Bool(_) => "Bool",
            Self::Text(GtkTextPropertySetter::BoxOrientation) => {
                "text naming a valid Orientation value"
            }
            Self::Text(_) => "Text",
            Self::TextOrI64(_) => "Int or integer text",
            Self::I64(_) => "Int",
            Self::F64(_) => "Float",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GtkPropertyDescriptor {
    pub name: &'static str,
    pub value_shape: GtkPropertyValueShape,
    pub setter: GtkPropertySetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkEventSignal {
    ButtonClicked,
    EntryActivated,
    CheckButtonToggled,
    ToggleButtonToggled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GtkEventDescriptor {
    pub name: &'static str,
    pub payload: GtkConcreteEventPayload,
    pub signal: GtkEventSignal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkChildContainerKind {
    Single,
    Sequence,
}

impl GtkChildContainerKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Single => "single-child",
            Self::Sequence => "append-only",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkChildMountRoute {
    WindowContent,
    BoxChildren,
    ScrolledWindowContent,
    RevealerChild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GtkChildGroupDescriptor {
    pub name: &'static str,
    pub container: GtkChildContainerKind,
    pub mount: GtkChildMountRoute,
    pub min_children: usize,
    pub max_children: Option<usize>,
}

impl GtkChildGroupDescriptor {
    pub const fn accepts_child_count(self, count: usize) -> bool {
        if count < self.min_children {
            return false;
        }
        match self.max_children {
            Some(max_children) => count <= max_children,
            None => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkDefaultChildGroup {
    None,
    One(&'static GtkChildGroupDescriptor),
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GtkWidgetSchema {
    pub markup_name: &'static str,
    pub kind: GtkConcreteWidgetKind,
    pub root_kind: GtkWidgetRootKind,
    pub properties: &'static [GtkPropertyDescriptor],
    pub events: &'static [GtkEventDescriptor],
    pub child_groups: &'static [GtkChildGroupDescriptor],
}

impl GtkWidgetSchema {
    pub fn property(&self, name: &str) -> Option<&GtkPropertyDescriptor> {
        self.properties
            .iter()
            .find(|descriptor| descriptor.name == name)
    }

    pub fn event(&self, name: &str) -> Option<&GtkEventDescriptor> {
        self.events
            .iter()
            .find(|descriptor| descriptor.name == name)
    }

    pub fn default_child_group(&self) -> GtkDefaultChildGroup {
        match self.child_groups {
            [] => GtkDefaultChildGroup::None,
            [group] => GtkDefaultChildGroup::One(group),
            _ => GtkDefaultChildGroup::Ambiguous,
        }
    }

    pub const fn is_window_root(&self) -> bool {
        self.root_kind.is_window()
    }
}

const ORIENTATION_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "Orientation",
    variants: &["Vertical", "Horizontal"],
};

const VISIBLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "visible",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Visible),
};

const SENSITIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "sensitive",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Sensitive),
};

const HEXPAND_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "hexpand",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Hexpand),
};

const VEXPAND_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "vexpand",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Vexpand),
};

const WINDOW_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::WindowTitle),
};

const BOX_ORIENTATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "orientation",
    value_shape: GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::BoxOrientation),
};

const BOX_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "spacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::TextOrI64(GtkTextOrI64PropertySetter::BoxSpacing),
};

const ENTRY_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntryText),
};

const ENTRY_PLACEHOLDER_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "placeholderText",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntryPlaceholderText),
};

const ENTRY_EDITABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "editable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::EntryEditable),
};

const LABEL_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::LabelText),
};

const LABEL_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::LabelLabel),
};

const BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ButtonLabel),
};

const BUTTON_CLICK_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onClick",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::ButtonClicked,
};

const ENTRY_ACTIVATE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivate",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::EntryActivated,
};

const SWITCH_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SwitchActive),
};

const WINDOW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::WindowContent,
    min_children: 0,
    max_children: Some(1),
};

const BOX_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::BoxChildren,
    min_children: 0,
    max_children: None,
};

const SCROLLED_WINDOW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ScrolledWindowContent,
    min_children: 0,
    max_children: Some(1),
};

const WINDOW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Window",
    kind: GtkConcreteWidgetKind::Window,
    root_kind: GtkWidgetRootKind::Window,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        WINDOW_TITLE_PROPERTY,
    ],
    events: &[],
    child_groups: &[WINDOW_CONTENT_CHILD_GROUP],
};

const BOX_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Box",
    kind: GtkConcreteWidgetKind::Box,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        BOX_ORIENTATION_PROPERTY,
        BOX_SPACING_PROPERTY,
    ],
    events: &[],
    child_groups: &[BOX_CHILDREN_CHILD_GROUP],
};

const SCROLLED_WINDOW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ScrolledWindow",
    kind: GtkConcreteWidgetKind::ScrolledWindow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
    ],
    events: &[],
    child_groups: &[SCROLLED_WINDOW_CONTENT_CHILD_GROUP],
};

const LABEL_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Label",
    kind: GtkConcreteWidgetKind::Label,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        LABEL_TEXT_PROPERTY,
        LABEL_LABEL_PROPERTY,
    ],
    events: &[],
    child_groups: &[],
};

const BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Button",
    kind: GtkConcreteWidgetKind::Button,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        BUTTON_LABEL_PROPERTY,
    ],
    events: &[BUTTON_CLICK_EVENT],
    child_groups: &[],
};

const ENTRY_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Entry",
    kind: GtkConcreteWidgetKind::Entry,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        ENTRY_TEXT_PROPERTY,
        ENTRY_PLACEHOLDER_TEXT_PROPERTY,
        ENTRY_EDITABLE_PROPERTY,
    ],
    events: &[ENTRY_ACTIVATE_EVENT],
    child_groups: &[],
};

const SWITCH_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Switch",
    kind: GtkConcreteWidgetKind::Switch,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        SWITCH_ACTIVE_PROPERTY,
    ],
    events: &[],
    child_groups: &[],
};

const CHECK_BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::CheckButtonLabel),
};

const CHECK_BUTTON_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::CheckButtonActive),
};

const CHECK_BUTTON_TOGGLE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onToggle",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::CheckButtonToggled,
};

const CHECK_BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "CheckButton",
    kind: GtkConcreteWidgetKind::CheckButton,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        CHECK_BUTTON_LABEL_PROPERTY,
        CHECK_BUTTON_ACTIVE_PROPERTY,
    ],
    events: &[CHECK_BUTTON_TOGGLE_EVENT],
    child_groups: &[],
};

const TOGGLE_BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ToggleButtonLabel),
};

const TOGGLE_BUTTON_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ToggleButtonActive),
};

const TOGGLE_BUTTON_TOGGLE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onToggle",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::ToggleButtonToggled,
};

const TOGGLE_BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ToggleButton",
    kind: GtkConcreteWidgetKind::ToggleButton,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        TOGGLE_BUTTON_LABEL_PROPERTY,
        TOGGLE_BUTTON_ACTIVE_PROPERTY,
    ],
    events: &[TOGGLE_BUTTON_TOGGLE_EVENT],
    child_groups: &[],
};

const IMAGE_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ImageIconName),
};

const IMAGE_RESOURCE_PATH_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "resourcePath",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ImageResourcePath),
};

const IMAGE_PIXEL_SIZE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "pixelSize",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::ImagePixelSize),
};

const IMAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Image",
    kind: GtkConcreteWidgetKind::Image,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        IMAGE_ICON_NAME_PROPERTY,
        IMAGE_RESOURCE_PATH_PROPERTY,
        IMAGE_PIXEL_SIZE_PROPERTY,
    ],
    events: &[],
    child_groups: &[],
};

const SPINNER_SPINNING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "spinning",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinnerSpinning),
};

const SPINNER_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Spinner",
    kind: GtkConcreteWidgetKind::Spinner,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        SPINNER_SPINNING_PROPERTY,
    ],
    events: &[],
    child_groups: &[],
};

const PROGRESS_BAR_FRACTION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "fraction",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ProgressBarFraction),
};

const PROGRESS_BAR_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ProgressBarText),
};

const PROGRESS_BAR_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ProgressBar",
    kind: GtkConcreteWidgetKind::ProgressBar,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        PROGRESS_BAR_FRACTION_PROPERTY,
        PROGRESS_BAR_TEXT_PROPERTY,
    ],
    events: &[],
    child_groups: &[],
};

const REVEALER_REVEALED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "revealed",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::RevealerRevealed),
};

const REVEALER_TRANSITION_TYPE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "transitionType",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::RevealerTransitionType),
};

const REVEALER_TRANSITION_DURATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "transitionDuration",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::RevealerTransitionDuration),
};

const REVEALER_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "child",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::RevealerChild,
    min_children: 0,
    max_children: Some(1),
};

const REVEALER_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Revealer",
    kind: GtkConcreteWidgetKind::Revealer,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        REVEALER_REVEALED_PROPERTY,
        REVEALER_TRANSITION_TYPE_PROPERTY,
        REVEALER_TRANSITION_DURATION_PROPERTY,
    ],
    events: &[],
    child_groups: &[REVEALER_CHILD_GROUP],
};

const GTK_WIDGET_SCHEMAS: &[GtkWidgetSchema] = &[
    WINDOW_SCHEMA,
    BOX_SCHEMA,
    SCROLLED_WINDOW_SCHEMA,
    LABEL_SCHEMA,
    BUTTON_SCHEMA,
    ENTRY_SCHEMA,
    SWITCH_SCHEMA,
    CHECK_BUTTON_SCHEMA,
    TOGGLE_BUTTON_SCHEMA,
    IMAGE_SCHEMA,
    SPINNER_SCHEMA,
    PROGRESS_BAR_SCHEMA,
    REVEALER_SCHEMA,
];

pub fn supported_widget_schemas() -> &'static [GtkWidgetSchema] {
    GTK_WIDGET_SCHEMAS
}

pub fn lookup_widget_schema(widget: &NamePath) -> Option<&'static GtkWidgetSchema> {
    lookup_widget_schema_by_name(widget_leaf_name(widget))
}

pub fn lookup_widget_schema_by_name(name: &str) -> Option<&'static GtkWidgetSchema> {
    supported_widget_schemas()
        .iter()
        .find(|schema| schema.markup_name == name)
}

pub fn lookup_widget_property(
    widget: &NamePath,
    property: &str,
) -> Option<&'static GtkPropertyDescriptor> {
    lookup_widget_schema(widget)?.property(property)
}

pub fn lookup_widget_event(widget: &NamePath, event: &str) -> Option<&'static GtkEventDescriptor> {
    lookup_widget_schema(widget)?.event(event)
}

pub fn concrete_widget_is_window(widget: &NamePath) -> bool {
    lookup_widget_schema(widget).is_some_and(|schema| schema.is_window_root())
}

pub fn concrete_supports_property(widget: &NamePath, property: &str) -> bool {
    lookup_widget_property(widget, property).is_some()
}

pub fn concrete_event_payload(widget: &NamePath, event: &str) -> Option<GtkConcreteEventPayload> {
    lookup_widget_event(widget, event).map(|descriptor| descriptor.payload)
}

pub(crate) fn widget_leaf_name(path: &NamePath) -> &str {
    path.segments()
        .iter()
        .last()
        .expect("NamePath must contain at least one segment — this is a parser invariant")
        .text()
}

#[cfg(test)]
mod tests {
    use aivi_base::{FileId, SourceSpan, Span};
    use aivi_hir::{Name, NamePath};

    use super::{
        GtkChildContainerKind, GtkConcreteEventPayload, GtkDefaultChildGroup,
        GtkPropertyValueShape, lookup_widget_event, lookup_widget_property, lookup_widget_schema,
        lookup_widget_schema_by_name, supported_widget_schemas,
    };

    fn span() -> SourceSpan {
        SourceSpan::new(FileId::new(0), Span::from(0..0))
    }

    fn name(text: &str) -> Name {
        Name::new(text, span()).expect("test names should be valid")
    }

    fn path(segments: &[&str]) -> NamePath {
        NamePath::from_vec(segments.iter().map(|segment| name(segment)).collect())
            .expect("test paths should be valid")
    }

    #[test]
    fn catalog_lists_current_supported_widget_surface() {
        let names = supported_widget_schemas()
            .iter()
            .map(|schema| schema.markup_name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Window",
                "Box",
                "ScrolledWindow",
                "Label",
                "Button",
                "Entry",
                "Switch",
                "CheckButton",
                "ToggleButton",
                "Image",
                "Spinner",
                "ProgressBar",
                "Revealer",
            ]
        );
    }

    #[test]
    fn lookup_uses_the_leaf_segment_for_current_widget_names() {
        let qualified = path(&["gtk", "Button"]);
        let schema = lookup_widget_schema(&qualified).expect("leaf lookup should resolve Button");
        assert_eq!(schema.markup_name, "Button");
    }

    #[test]
    fn property_descriptors_are_exact_and_widget_specific() {
        let button = path(&["Button"]);
        let entry = path(&["Entry"]);
        let label = path(&["Label"]);
        let switch = path(&["Switch"]);
        let property = lookup_widget_property(&button, "label")
            .expect("Button.label should be part of the catalog");
        assert_eq!(property.value_shape, GtkPropertyValueShape::Text);
        assert!(lookup_widget_property(&button, "text").is_none());
        assert!(lookup_widget_property(&label, "label").is_some());
        assert!(lookup_widget_property(&entry, "text").is_some());
        assert!(lookup_widget_property(&entry, "placeholderText").is_some());
        assert!(lookup_widget_property(&entry, "label").is_none());
        assert!(lookup_widget_property(&switch, "active").is_some());
        assert!(lookup_widget_property(&switch, "text").is_none());
    }

    #[test]
    fn event_descriptors_are_exact_and_case_sensitive() {
        let button = path(&["Button"]);
        let entry = path(&["Entry"]);
        let event =
            lookup_widget_event(&button, "onClick").expect("Button.onClick should be in catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Unit);
        let event = lookup_widget_event(&entry, "onActivate")
            .expect("Entry.onActivate should be part of the catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Unit);
        assert!(lookup_widget_event(&button, "onclick").is_none());
        assert!(lookup_widget_event(&entry, "onactivate").is_none());
        assert!(lookup_widget_event(&path(&["Label"]), "onClick").is_none());
    }

    #[test]
    fn child_group_metadata_tracks_container_policy() {
        let window = lookup_widget_schema_by_name("Window").expect("Window schema should exist");
        assert!(matches!(
            window.default_child_group(),
            GtkDefaultChildGroup::One(group)
                if group.name == "content"
                    && group.container == GtkChildContainerKind::Single
                    && group.accepts_child_count(1)
                    && !group.accepts_child_count(2)
        ));

        let scrolled_window = lookup_widget_schema_by_name("ScrolledWindow")
            .expect("ScrolledWindow schema should exist");
        assert!(matches!(
            scrolled_window.default_child_group(),
            GtkDefaultChildGroup::One(group)
                if group.name == "content"
                    && group.container == GtkChildContainerKind::Single
                    && group.accepts_child_count(1)
                    && !group.accepts_child_count(2)
        ));

        let button = lookup_widget_schema_by_name("Button").expect("Button schema should exist");
        assert_eq!(button.default_child_group(), GtkDefaultChildGroup::None);
    }
}
