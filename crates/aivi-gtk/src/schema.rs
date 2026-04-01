use aivi_hir::NamePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteEventPayload {
    Unit,
    Bool,
    Text,
}

impl GtkConcreteEventPayload {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unit => "Unit",
            Self::Bool => "Bool",
            Self::Text => "Text",
        }
    }

    pub const fn required_signal_type_label(self) -> &'static str {
        match self {
            Self::Unit => "`Signal Unit`",
            Self::Bool => "`Signal Bool`",
            Self::Text => "`Signal Text`",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteWidgetKind {
    Window,
    HeaderBar,
    Paned,
    Box,
    ScrolledWindow,
    Frame,
    Viewport,
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
    Separator,
}

impl GtkConcreteWidgetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::HeaderBar => "HeaderBar",
            Self::Paned => "Paned",
            Self::Box => "Box",
            Self::ScrolledWindow => "ScrolledWindow",
            Self::Frame => "Frame",
            Self::Viewport => "Viewport",
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
            Self::Separator => "Separator",
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
    Monospace,
    ButtonCompact,
    ButtonHasFrame,
    HeaderBarShowTitleButtons,
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
    FrameLabel,
    LabelText,
    LabelLabel,
    ButtonLabel,
    BoxOrientation,
    PanedOrientation,
    SeparatorOrientation,
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
    WidthRequest,
    HeightRequest,
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
            Self::Text(
                GtkTextPropertySetter::BoxOrientation
                | GtkTextPropertySetter::PanedOrientation
                | GtkTextPropertySetter::SeparatorOrientation,
            ) => "text naming a valid Orientation value",
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
    EntryChanged,
    EntryActivated,
    SwitchToggled,
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
    WindowTitlebar,
    HeaderBarTitleWidget,
    HeaderBarStart,
    HeaderBarEnd,
    PanedStart,
    PanedEnd,
    BoxChildren,
    ScrolledWindowContent,
    FrameChild,
    ViewportChild,
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
    pub default_child_group_override: Option<&'static GtkChildGroupDescriptor>,
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

    pub fn child_group(&self, name: &str) -> Option<&'static GtkChildGroupDescriptor> {
        self.child_groups
            .iter()
            .find(|descriptor| descriptor.name == name)
    }

    pub fn default_child_group(&self) -> GtkDefaultChildGroup {
        if let Some(group) = self.default_child_group_override {
            GtkDefaultChildGroup::One(group)
        } else {
            match self.child_groups {
                [] => GtkDefaultChildGroup::None,
                [group] => GtkDefaultChildGroup::One(group),
                _ => GtkDefaultChildGroup::Ambiguous,
            }
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

const MONOSPACE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "monospace",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Monospace),
};

const BUTTON_COMPACT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "compact",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonCompact),
};

const BUTTON_HAS_FRAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "hasFrame",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonHasFrame),
};

const WIDTH_REQUEST_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "widthRequest",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::WidthRequest),
};

const HEIGHT_REQUEST_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "heightRequest",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::HeightRequest),
};

const WINDOW_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::WindowTitle),
};

const HEADER_BAR_SHOW_TITLE_BUTTONS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showTitleButtons",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::HeaderBarShowTitleButtons),
};

const BOX_ORIENTATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "orientation",
    value_shape: GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::BoxOrientation),
};

const PANED_ORIENTATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "orientation",
    value_shape: GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PanedOrientation),
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

const FRAME_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FrameLabel),
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

const ENTRY_CHANGE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChange",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::EntryChanged,
};

const SWITCH_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SwitchActive),
};

const SWITCH_TOGGLE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onToggle",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::SwitchToggled,
};

const WINDOW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::WindowContent,
    min_children: 0,
    max_children: Some(1),
};

const WINDOW_TITLEBAR_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "titlebar",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::WindowTitlebar,
    min_children: 0,
    max_children: Some(1),
};

const HEADER_BAR_TITLE_WIDGET_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "titleWidget",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::HeaderBarTitleWidget,
    min_children: 0,
    max_children: Some(1),
};

const HEADER_BAR_START_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "start",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::HeaderBarStart,
    min_children: 0,
    max_children: None,
};

const HEADER_BAR_END_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "end",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::HeaderBarEnd,
    min_children: 0,
    max_children: None,
};

const PANED_START_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "start",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::PanedStart,
    min_children: 0,
    max_children: Some(1),
};

const PANED_END_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "end",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::PanedEnd,
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

const FRAME_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "child",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::FrameChild,
    min_children: 0,
    max_children: Some(1),
};

const VIEWPORT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "child",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ViewportChild,
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
    default_child_group_override: Some(&WINDOW_CONTENT_CHILD_GROUP),
    child_groups: &[WINDOW_CONTENT_CHILD_GROUP, WINDOW_TITLEBAR_CHILD_GROUP],
};

const HEADER_BAR_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "HeaderBar",
    kind: GtkConcreteWidgetKind::HeaderBar,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        HEADER_BAR_SHOW_TITLE_BUTTONS_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[
        HEADER_BAR_START_CHILD_GROUP,
        HEADER_BAR_END_CHILD_GROUP,
        HEADER_BAR_TITLE_WIDGET_CHILD_GROUP,
    ],
};

const PANED_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Paned",
    kind: GtkConcreteWidgetKind::Paned,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        PANED_ORIENTATION_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[PANED_START_CHILD_GROUP, PANED_END_CHILD_GROUP],
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
    default_child_group_override: None,
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
    default_child_group_override: None,
    child_groups: &[SCROLLED_WINDOW_CONTENT_CHILD_GROUP],
};

const FRAME_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Frame",
    kind: GtkConcreteWidgetKind::Frame,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        FRAME_LABEL_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[FRAME_CHILD_GROUP],
};

const VIEWPORT_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Viewport",
    kind: GtkConcreteWidgetKind::Viewport,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[VIEWPORT_CHILD_GROUP],
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
        MONOSPACE_PROPERTY,
        LABEL_TEXT_PROPERTY,
        LABEL_LABEL_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
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
        BUTTON_COMPACT_PROPERTY,
        BUTTON_HAS_FRAME_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        BUTTON_LABEL_PROPERTY,
    ],
    events: &[BUTTON_CLICK_EVENT],
    default_child_group_override: None,
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
    events: &[ENTRY_CHANGE_EVENT, ENTRY_ACTIVATE_EVENT],
    default_child_group_override: None,
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
    events: &[SWITCH_TOGGLE_EVENT],
    default_child_group_override: None,
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
    default_child_group_override: None,
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
    default_child_group_override: None,
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
    default_child_group_override: None,
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
    default_child_group_override: None,
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
    default_child_group_override: None,
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
    default_child_group_override: None,
    child_groups: &[REVEALER_CHILD_GROUP],
};

const SEPARATOR_ORIENTATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "orientation",
    value_shape: GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::SeparatorOrientation),
};

const SEPARATOR_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Separator",
    kind: GtkConcreteWidgetKind::Separator,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        SEPARATOR_ORIENTATION_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

const GTK_WIDGET_SCHEMAS: &[GtkWidgetSchema] = &[
    WINDOW_SCHEMA,
    HEADER_BAR_SCHEMA,
    PANED_SCHEMA,
    BOX_SCHEMA,
    SCROLLED_WINDOW_SCHEMA,
    FRAME_SCHEMA,
    VIEWPORT_SCHEMA,
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
    SEPARATOR_SCHEMA,
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
        GtkPropertyValueShape, ORIENTATION_VALUE_SHAPE, lookup_widget_event,
        lookup_widget_property, lookup_widget_schema, lookup_widget_schema_by_name,
        supported_widget_schemas,
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
                "HeaderBar",
                "Paned",
                "Box",
                "ScrolledWindow",
                "Frame",
                "Viewport",
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
                "Separator",
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
        let header_bar = path(&["HeaderBar"]);
        let separator = path(&["Separator"]);
        let property = lookup_widget_property(&button, "label")
            .expect("Button.label should be part of the catalog");
        assert_eq!(property.value_shape, GtkPropertyValueShape::Text);
        assert!(lookup_widget_property(&button, "text").is_none());
        assert!(lookup_widget_property(&button, "compact").is_some());
        assert!(lookup_widget_property(&button, "hasFrame").is_some());
        assert!(lookup_widget_property(&button, "widthRequest").is_some());
        assert!(lookup_widget_property(&button, "heightRequest").is_some());
        assert!(lookup_widget_property(&label, "label").is_some());
        assert!(lookup_widget_property(&label, "monospace").is_some());
        assert!(lookup_widget_property(&entry, "text").is_some());
        assert!(lookup_widget_property(&entry, "placeholderText").is_some());
        assert!(lookup_widget_property(&entry, "label").is_none());
        assert!(lookup_widget_property(&switch, "active").is_some());
        assert!(lookup_widget_property(&header_bar, "showTitleButtons").is_some());
        assert_eq!(
            lookup_widget_property(&separator, "orientation")
                .expect("Separator.orientation should be part of the catalog")
                .value_shape,
            GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE)
        );
        assert!(lookup_widget_property(&switch, "text").is_none());
    }

    #[test]
    fn event_descriptors_are_exact_and_case_sensitive() {
        let button = path(&["Button"]);
        let entry = path(&["Entry"]);
        let switch = path(&["Switch"]);
        let event =
            lookup_widget_event(&button, "onClick").expect("Button.onClick should be in catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Unit);
        let event = lookup_widget_event(&entry, "onChange")
            .expect("Entry.onChange should be part of the catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Text);
        let event = lookup_widget_event(&entry, "onActivate")
            .expect("Entry.onActivate should be part of the catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Unit);
        let event =
            lookup_widget_event(&switch, "onToggle").expect("Switch.onToggle should be in catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Bool);
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
        let titlebar = window
            .child_group("titlebar")
            .expect("Window should expose an explicit titlebar group");
        assert_eq!(titlebar.container, GtkChildContainerKind::Single);
        assert!(titlebar.accepts_child_count(1));
        assert!(!titlebar.accepts_child_count(2));

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

        let frame = lookup_widget_schema_by_name("Frame").expect("Frame schema should exist");
        assert!(matches!(
            frame.default_child_group(),
            GtkDefaultChildGroup::One(group)
                if group.name == "child"
                    && group.container == GtkChildContainerKind::Single
                    && group.accepts_child_count(1)
                    && !group.accepts_child_count(2)
        ));

        let header_bar =
            lookup_widget_schema_by_name("HeaderBar").expect("HeaderBar schema should exist");
        assert_eq!(
            header_bar.default_child_group(),
            GtkDefaultChildGroup::Ambiguous
        );
        let title_widget = header_bar
            .child_group("titleWidget")
            .expect("HeaderBar should expose an explicit titleWidget group");
        assert_eq!(title_widget.container, GtkChildContainerKind::Single);
        assert!(title_widget.accepts_child_count(1));
        assert!(!title_widget.accepts_child_count(2));
    }
}
