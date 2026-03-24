use aivi_hir::NamePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteEventPayload {
    Unit,
}

impl GtkConcreteEventPayload {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unit => "Unit",
        }
    }

    pub const fn required_signal_type_label(self) -> &'static str {
        match self {
            Self::Unit => "`Signal Unit`",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteWidgetKind {
    Window,
    Box,
    Label,
    Button,
}

impl GtkConcreteWidgetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::Box => "Box",
            Self::Label => "Label",
            Self::Button => "Button",
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
    Enum(GtkEnumValueShape),
}

impl GtkPropertyValueShape {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Text => "Text",
            Self::I64 => "Int",
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkTextPropertySetter {
    WindowTitle,
    LabelText,
    LabelLabel,
    ButtonLabel,
    BoxOrientation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkTextOrI64PropertySetter {
    BoxSpacing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkPropertySetter {
    Bool(GtkBoolPropertySetter),
    Text(GtkTextPropertySetter),
    TextOrI64(GtkTextOrI64PropertySetter),
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

const GTK_WIDGET_SCHEMAS: &[GtkWidgetSchema] =
    &[WINDOW_SCHEMA, BOX_SCHEMA, LABEL_SCHEMA, BUTTON_SCHEMA];

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
        assert_eq!(names, ["Window", "Box", "Label", "Button"]);
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
        let label = path(&["Label"]);
        let property = lookup_widget_property(&button, "label")
            .expect("Button.label should be part of the catalog");
        assert_eq!(property.value_shape, GtkPropertyValueShape::Text);
        assert!(lookup_widget_property(&button, "text").is_none());
        assert!(lookup_widget_property(&label, "label").is_some());
    }

    #[test]
    fn event_descriptors_are_exact_and_case_sensitive() {
        let button = path(&["Button"]);
        let event =
            lookup_widget_event(&button, "onClick").expect("Button.onClick should be in catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::Unit);
        assert!(lookup_widget_event(&button, "onclick").is_none());
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

        let button = lookup_widget_schema_by_name("Button").expect("Button schema should exist");
        assert_eq!(button.default_child_group(), GtkDefaultChildGroup::None);
    }
}
