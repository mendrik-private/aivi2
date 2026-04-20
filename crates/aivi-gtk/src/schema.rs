use aivi_hir::NamePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkConcreteEventPayload {
    Unit,
    Bool,
    Text,
    F64,
    I64,
}

impl GtkConcreteEventPayload {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unit => "Unit",
            Self::Bool => "Bool",
            Self::Text => "Text",
            Self::F64 => "Float",
            Self::I64 => "Int",
        }
    }

    pub const fn required_signal_type_label(self) -> &'static str {
        match self {
            Self::Unit => "`Signal Unit`",
            Self::Bool => "`Signal Bool`",
            Self::Text => "`Signal Text`",
            Self::F64 => "`Signal Float`",
            Self::I64 => "`Signal Int`",
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
    SpinButton,
    Scale,
    Image,
    Spinner,
    ProgressBar,
    Revealer,
    Separator,
    StatusPage,
    Clamp,
    Banner,
    ToolbarView,
    // Group A: Adwaita preference rows
    ActionRow,
    ExpanderRow,
    SwitchRow,
    SpinRow,
    EntryRow,
    // Group B: List and selection
    ListBox,
    ListBoxRow,
    ListView,
    GridView,
    DropDown,
    // Group C: Utility
    SearchEntry,
    Expander,
    // Group D: Navigation and overlay
    NavigationView,
    NavigationPage,
    ToastOverlay,
    // Group E: Adwaita preferences structure
    PreferencesGroup,
    PreferencesPage,
    PreferencesWindow,
    ComboRow,
    PasswordEntryRow,
    // Group F: Layout
    Overlay,
    // Group G: Input
    MultilineEntry,
    // Group H: Picture
    Picture,
    WebView,
    // Group I: ViewStack navigation
    ViewStack,
    ViewStackPage,
    // Group J: Dialogs
    AlertDialog,
    // Group K: Interactive and layout widgets
    Calendar,
    FlowBox,
    FlowBoxChild,
    MenuButton,
    Popover,
    // Group L: Navigation and layout (Tier 1)
    CenterBox,
    AboutDialog,
    SplitButton,
    NavigationSplitView,
    OverlaySplitView,
    TabView,
    TabPage,
    TabBar,
    Carousel,
    CarouselIndicatorDots,
    CarouselIndicatorLines,
    Grid,
    GridChild,
    FileDialog,
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
            Self::SpinButton => "SpinButton",
            Self::Scale => "Scale",
            Self::Image => "Image",
            Self::Spinner => "Spinner",
            Self::ProgressBar => "ProgressBar",
            Self::Revealer => "Revealer",
            Self::Separator => "Separator",
            Self::StatusPage => "StatusPage",
            Self::Clamp => "Clamp",
            Self::Banner => "Banner",
            Self::ToolbarView => "ToolbarView",
            Self::ActionRow => "ActionRow",
            Self::ExpanderRow => "ExpanderRow",
            Self::SwitchRow => "SwitchRow",
            Self::SpinRow => "SpinRow",
            Self::EntryRow => "EntryRow",
            Self::ListBox => "ListBox",
            Self::ListBoxRow => "ListBoxRow",
            Self::ListView => "ListView",
            Self::GridView => "GridView",
            Self::DropDown => "DropDown",
            Self::SearchEntry => "SearchEntry",
            Self::Expander => "Expander",
            Self::NavigationView => "NavigationView",
            Self::NavigationPage => "NavigationPage",
            Self::ToastOverlay => "ToastOverlay",
            Self::PreferencesGroup => "PreferencesGroup",
            Self::PreferencesPage => "PreferencesPage",
            Self::PreferencesWindow => "PreferencesWindow",
            Self::ComboRow => "ComboRow",
            Self::PasswordEntryRow => "PasswordEntryRow",
            Self::Overlay => "Overlay",
            Self::MultilineEntry => "MultilineEntry",
            Self::Picture => "Picture",
            Self::WebView => "WebView",
            Self::ViewStack => "ViewStack",
            Self::ViewStackPage => "ViewStackPage",
            Self::AlertDialog => "AlertDialog",
            Self::Calendar => "Calendar",
            Self::FlowBox => "FlowBox",
            Self::FlowBoxChild => "FlowBoxChild",
            Self::MenuButton => "MenuButton",
            Self::Popover => "Popover",
            Self::CenterBox => "CenterBox",
            Self::AboutDialog => "AboutDialog",
            Self::SplitButton => "SplitButton",
            Self::NavigationSplitView => "NavigationSplitView",
            Self::OverlaySplitView => "OverlaySplitView",
            Self::TabView => "TabView",
            Self::TabPage => "TabPage",
            Self::TabBar => "TabBar",
            Self::Carousel => "Carousel",
            Self::CarouselIndicatorDots => "CarouselIndicatorDots",
            Self::CarouselIndicatorLines => "CarouselIndicatorLines",
            Self::Grid => "Grid",
            Self::GridChild => "GridChild",
            Self::FileDialog => "FileDialog",
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
    Focusable,
    Hexpand,
    Vexpand,
    AnimateOpacity,
    Monospace,
    ButtonCompact,
    ButtonHasFrame,
    HeaderBarShowTitleButtons,
    EntryEditable,
    SwitchActive,
    CheckButtonActive,
    ToggleButtonActive,
    SpinButtonWrap,
    SpinButtonNumeric,
    ScaleDrawValue,
    SpinnerSpinning,
    RevealerRevealed,
    WindowResizable,
    WindowModal,
    LabelWrap,
    LabelSelectable,
    LabelUseMarkup,
    EntryVisibility,
    ScrolledWindowPropagateNaturalWidth,
    ScrolledWindowPropagateNaturalHeight,
    ProgressBarShowText,
    BoxHomogeneous,
    BannerRevealed,
    // Group A: Adwaita preference rows
    ExpanderRowExpanded,
    SwitchRowActive,
    // Shared for ActionRow and ListBoxRow (both are ListBoxRow subtypes)
    ListBoxRowActivatable,
    // Group C: Utility
    ExpanderExpanded,
    // Group E: Preferences structure
    PreferencesWindowSearchEnabled,
    // Phase 4: Button extras
    ButtonUseUnderline,
    // Group G: MultilineEntry
    MultilineEntryEditable,
    MultilineEntryMonospace,
    // Group H: Picture
    PictureCanShrink,
    // Window state
    WindowMaximized,
    WindowFullscreen,
    WindowDecorated,
    WindowHideOnClose,
    // Button extra
    ButtonReceivesDefault,
    // Label extra
    LabelSingleLineMode,
    // MenuButton
    MenuButtonActive,
    MenuButtonUseUnderline,
    // Popover
    PopoverAutohide,
    PopoverHasArrow,
    // Tier 1 additions
    AboutDialogVisible,
    FileDialogVisible,
    NavigationSplitViewShowContent,
    OverlaySplitViewShowSidebar,
    TabPageNeedsAttention,
    TabPageLoading,
    TabBarAutohide,
    TabBarExpandTabs,
    CarouselInteractive,
    GridRowHomogeneous,
    GridColumnHomogeneous,
    ListBoxShowSeparators,
    ListViewShowSeparators,
    ListViewEnableRubberband,
    ListViewSingleClickActivate,
    GridViewEnableRubberband,
    GridViewSingleClickActivate,
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
    ScaleOrientation,
    Halign,
    Valign,
    Tooltip,
    CssClasses,
    LabelWrapMode,
    LabelJustify,
    LabelEllipsize,
    EntryInputPurpose,
    ScrolledWindowHPolicy,
    ScrolledWindowVPolicy,
    ImageFile,
    StatusPageTitle,
    StatusPageDescription,
    StatusPageIconName,
    BannerTitle,
    BannerButtonLabel,
    // Group A: Adwaita preference rows — shared title via PreferencesRowExt
    AdwPreferencesRowTitle,
    // Subtitle: ActionRow, SwitchRow, SpinRow share ActionRowExt::set_subtitle
    AdwActionRowSubtitle,
    // Subtitle: ExpanderRow uses ExpanderRowExt::set_subtitle
    AdwExpanderRowSubtitle,
    // EntryRow text content via EditableExt
    EntryRowText,
    // Group B: List and selection
    ListBoxSelectionMode,
    DropDownItems,
    // Group C: Utility
    SearchEntryText,
    SearchEntryPlaceholder,
    ExpanderLabel,
    // Group D: Navigation
    NavigationPageTitle,
    NavigationPageTag,
    // Group E: Preferences structure
    PreferencesGroupTitle,
    PreferencesGroupDescription,
    PreferencesPageTitle,
    PreferencesPageIconName,
    ComboRowItems,
    PasswordEntryRowText,
    // Phase 4: Button extras
    ButtonIconName,
    // Group G: MultilineEntry
    MultilineEntryText,
    MultilineEntryWrapMode,
    // Group H: Picture
    PictureFilename,
    PictureResource,
    PictureContentFit,
    PictureAltText,
    WebViewHtml,
    // Group I: ViewStack navigation
    ViewStackVisibleChild,
    ViewStackPageName,
    ViewStackPageTitle,
    ViewStackPageIconName,
    // Group J: AlertDialog (adw::MessageDialog)
    AlertDialogHeading,
    AlertDialogBody,
    AlertDialogDefaultResponse,
    AlertDialogCloseResponse,
    AlertDialogResponses,
    // HeaderBar extra
    HeaderBarDecorationLayout,
    // Scale extra
    ScaleValuePos,
    // FlowBox
    FlowBoxSelectionMode,
    // MenuButton
    MenuButtonLabel,
    MenuButtonIconName,
    // Tier 1 additions
    AboutDialogAppName,
    AboutDialogVersion,
    AboutDialogDeveloperName,
    AboutDialogComments,
    AboutDialogWebsite,
    AboutDialogIssueUrl,
    AboutDialogLicenseType,
    AboutDialogApplicationIcon,
    TabPageTitle,
    SplitButtonLabel,
    SplitButtonIconName,
    NavigationSplitViewSidebarPosition,
    OverlaySplitViewSidebarPosition,
    FileDialogTitle,
    FileDialogMode,
    FileDialogAcceptLabel,
    FileDialogCancelLabel,
    EntryPrimaryIconName,
    EntrySecondaryIconName,
    HeaderBarCenteringPolicy,
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
    SpinButtonDigits,
    ScaleDigits,
    MarginStart,
    MarginEnd,
    MarginTop,
    MarginBottom,
    WindowDefaultWidth,
    WindowDefaultHeight,
    LabelMaxWidthChars,
    EntryMaxLength,
    ClampMaximumSize,
    ClampTighteningThreshold,
    // Group B: List and selection
    DropDownSelected,
    // Group E: ComboRow
    ComboRowSelected,
    // Phase 4: Label extras
    LabelLines,
    // Group G: MultilineEntry
    MultilineEntryTopMargin,
    MultilineEntryBottomMargin,
    MultilineEntryLeftMargin,
    MultilineEntryRightMargin,
    // Label extra
    LabelWidthChars,
    // Calendar
    CalendarYear,
    CalendarMonth,
    CalendarDay,
    // FlowBox
    FlowBoxRowSpacing,
    FlowBoxColumnSpacing,
    // Tier 1 additions
    GridRowSpacing,
    GridColumnSpacing,
    GridChildColumn,
    GridChildRow,
    GridChildColumnSpan,
    GridChildRowSpan,
    GridViewMinColumns,
    GridViewMaxColumns,
    CarouselSpacing,
    CarouselRevealDuration,
    TabViewSelectedPage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GtkF64PropertySetter {
    WidgetOpacity,
    ProgressBarFraction,
    SpinButtonValue,
    SpinButtonMin,
    SpinButtonMax,
    SpinButtonStep,
    ScaleValue,
    ScaleMin,
    ScaleMax,
    ScaleStep,
    // Group A: Adwaita preference rows
    SpinRowValue,
    SpinRowMin,
    SpinRowMax,
    SpinRowStep,
    // Scale extra
    ScaleFillLevel,
    // Label alignment
    LabelXalign,
    LabelYalign,
    // Tier 1 additions
    NavigationSplitViewSidebarWidthFraction,
    NavigationSplitViewMinSidebarWidth,
    NavigationSplitViewMaxSidebarWidth,
    OverlaySplitViewSidebarWidthFraction,
    OverlaySplitViewMinSidebarWidth,
    OverlaySplitViewMaxSidebarWidth,
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
                | GtkTextPropertySetter::SeparatorOrientation
                | GtkTextPropertySetter::ScaleOrientation,
            ) => "text naming a valid Orientation value",
            Self::Text(GtkTextPropertySetter::Halign | GtkTextPropertySetter::Valign) => {
                "text naming a valid Align value"
            }
            Self::Text(GtkTextPropertySetter::LabelWrapMode) => {
                "text naming a valid WrapMode value"
            }
            Self::Text(GtkTextPropertySetter::MultilineEntryWrapMode) => {
                "text naming a valid WrapMode value (None, Char, Word, WordChar)"
            }
            Self::Text(GtkTextPropertySetter::LabelJustify) => {
                "text naming a valid Justification value"
            }
            Self::Text(GtkTextPropertySetter::LabelEllipsize) => {
                "text naming a valid EllipsizeMode value"
            }
            Self::Text(GtkTextPropertySetter::EntryInputPurpose) => {
                "text naming a valid InputPurpose value"
            }
            Self::Text(
                GtkTextPropertySetter::ScrolledWindowHPolicy
                | GtkTextPropertySetter::ScrolledWindowVPolicy,
            ) => "text naming a valid PolicyType value",
            Self::Text(GtkTextPropertySetter::ListBoxSelectionMode) => {
                "text naming a valid SelectionMode value"
            }
            Self::Text(GtkTextPropertySetter::ScaleValuePos) => {
                "text naming a valid PositionType value (Top, Bottom, Left, Right)"
            }
            Self::Text(GtkTextPropertySetter::FlowBoxSelectionMode) => {
                "text naming a valid SelectionMode value"
            }
            Self::Text(GtkTextPropertySetter::HeaderBarCenteringPolicy) => {
                "text naming a valid CenteringPolicy value (Loose, Strict)"
            }
            Self::Text(
                GtkTextPropertySetter::NavigationSplitViewSidebarPosition
                | GtkTextPropertySetter::OverlaySplitViewSidebarPosition,
            ) => "text: Start or End",
            Self::Text(GtkTextPropertySetter::FileDialogMode) => {
                "text naming a valid FileChooserAction (Open, Save, OpenMultiple, SelectFolder)"
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
    EntryChanged,
    EntryActivated,
    SwitchToggled,
    CheckButtonToggled,
    ToggleButtonToggled,
    SpinButtonValueChanged,
    ScaleValueChanged,
    RevealerChildRevealed,
    FocusIn,
    FocusOut,
    Scroll,
    PointerEnter,
    PointerLeave,
    BannerButtonClicked,
    // Group A: Adwaita preference rows
    ActionRowActivated,
    SwitchRowToggled,
    SpinRowValueChanged,
    EntryRowChanged,
    EntryRowActivated,
    // Group B: List and selection
    ListBoxActivated,
    ListBoxRowActivated,
    ListViewActivated,
    GridViewActivated,
    DropDownSelectionChanged,
    // Group C: Utility
    SearchEntryChanged,
    SearchEntryActivated,
    SearchEntrySearchChanged,
    // Group E: ComboRow
    ComboRowSelectionChanged,
    // PasswordEntryRow
    PasswordEntryRowChanged,
    PasswordEntryRowActivated,
    // Group G: MultilineEntry
    MultilineEntryChanged,
    // Phase 4: Window
    WindowCloseRequest,
    // Phase 4: NavigationView
    NavigationViewPopped,
    // Expander events
    ExpanderRowExpanded,
    ExpanderExpanded,
    // NavigationPage events
    NavigationPageShowing,
    NavigationPageHiding,
    // ViewStack
    ViewStackSwitch,
    // AlertDialog (adw::MessageDialog)
    AlertDialogResponse,
    // Window state events
    WindowMaximized,
    WindowFullscreened,
    // Calendar events
    CalendarDaySelected,
    // FlowBox events
    FlowBoxChildActivated,
    // MenuButton event
    MenuButtonToggled,
    // Popover event
    PopoverClosed,
    // Tier 1 additions
    SecondaryClick,
    LongPress,
    SwipeLeft,
    SwipeRight,
    NavigationSplitViewShowContentChanged,
    OverlaySplitViewShowSidebarChanged,
    TabViewPageAdded,
    TabViewPageClosed,
    TabViewSelectedPageChanged,
    CarouselPageChanged,
    FileDialogResponse,
    SplitButtonClicked,
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
    StatusPageContent,
    ClampContent,
    ToolbarViewTop,
    ToolbarViewBottom,
    ToolbarViewContent,
    // Group A: Adwaita preference rows
    ActionRowSuffix,
    ExpanderRowRows,
    // Group B: List and selection
    ListBoxChildren,
    ListBoxRowChild,
    // Group C: Utility
    ExpanderChild,
    // Group D: Navigation and overlay
    NavigationViewPages,
    NavigationPageContent,
    ToastOverlayContent,
    // Group E: Preferences structure
    PreferencesGroupChildren,
    PreferencesPageChildren,
    PreferencesWindowPages,
    // Group F: Overlay
    OverlayContent,
    OverlayOverlay,
    // Group G: ViewStack navigation
    ViewStackPages,
    ViewStackPageContent,
    // FlowBox
    FlowBoxChildren,
    FlowBoxChildContent,
    ListViewChildren,
    GridViewChildren,
    // MenuButton
    MenuButtonPopover,
    // Popover
    PopoverContent,
    // Tier 1 additions
    CenterBoxStart,
    CenterBoxCenter,
    CenterBoxEnd,
    NavigationSplitViewSidebar,
    NavigationSplitViewContent,
    OverlaySplitViewSidebar,
    OverlaySplitViewContent,
    TabViewPages,
    TabViewTabBar,
    TabPageContent,
    CarouselPages,
    CarouselDots,
    CarouselLines,
    GridChildren,
    GridChildContent,
    ActionRowPrefix,
    SplitButtonPopover,
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

const FOCUSABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "focusable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::Focusable),
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

const ANIMATE_OPACITY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "animateOpacity",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::AnimateOpacity),
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

const BUTTON_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ButtonIconName),
};

const BUTTON_USE_UNDERLINE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "useUnderline",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonUseUnderline),
};

const BUTTON_RECEIVES_DEFAULT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "receivesDefault",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ButtonReceivesDefault),
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

const OPACITY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "opacity",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::WidgetOpacity),
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

const HEADER_BAR_DECORATION_LAYOUT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "decorationLayout",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::HeaderBarDecorationLayout),
};

const HEADER_BAR_CENTERING_POLICY_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "CenteringPolicy",
    variants: &["Loose", "Strict"],
};

const HEADER_BAR_CENTERING_POLICY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "centeringPolicy",
    value_shape: GtkPropertyValueShape::Enum(HEADER_BAR_CENTERING_POLICY_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::HeaderBarCenteringPolicy),
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

// ── Universal properties ─────────────────────────────────────────────────────

const ALIGN_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "Align",
    variants: &["Fill", "Start", "End", "Center", "Baseline"],
};

const HALIGN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "halign",
    value_shape: GtkPropertyValueShape::Enum(ALIGN_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::Halign),
};

const VALIGN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "valign",
    value_shape: GtkPropertyValueShape::Enum(ALIGN_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::Valign),
};

const MARGIN_START_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "marginStart",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MarginStart),
};

const MARGIN_END_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "marginEnd",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MarginEnd),
};

const MARGIN_TOP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "marginTop",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MarginTop),
};

const MARGIN_BOTTOM_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "marginBottom",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MarginBottom),
};

const TOOLTIP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "tooltip",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::Tooltip),
};

const CSS_CLASSES_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "cssClasses",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::CssClasses),
};

// ── Window-specific properties ───────────────────────────────────────────────

const WINDOW_DEFAULT_WIDTH_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "defaultWidth",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::WindowDefaultWidth),
};

const WINDOW_DEFAULT_HEIGHT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "defaultHeight",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::WindowDefaultHeight),
};

const WINDOW_RESIZABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "resizable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowResizable),
};

const WINDOW_MODAL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "modal",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowModal),
};

const WINDOW_CLOSE_REQUEST_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onCloseRequest",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::WindowCloseRequest,
};

const WINDOW_MAXIMIZED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "maximized",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowMaximized),
};

const WINDOW_FULLSCREEN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "fullscreen",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowFullscreen),
};

const WINDOW_DECORATED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "decorated",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowDecorated),
};

const WINDOW_HIDE_ON_CLOSE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "hideOnClose",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::WindowHideOnClose),
};

const WINDOW_MAXIMIZE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onMaximize",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::WindowMaximized,
};

const WINDOW_FULLSCREEN_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onFullscreen",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::WindowFullscreened,
};

// ── Label-specific properties ────────────────────────────────────────────────

const WRAP_MODE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "WrapMode",
    variants: &["Word", "Char", "WordChar"],
};

const JUSTIFICATION_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "Justification",
    variants: &["Left", "Center", "Right", "Fill"],
};

const ELLIPSIZE_MODE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "EllipsizeMode",
    variants: &["None", "Start", "Middle", "End"],
};

const LABEL_WRAP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "wrap",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelWrap),
};

const LABEL_WRAP_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "wrapMode",
    value_shape: GtkPropertyValueShape::Enum(WRAP_MODE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::LabelWrapMode),
};

const LABEL_JUSTIFY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "justify",
    value_shape: GtkPropertyValueShape::Enum(JUSTIFICATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::LabelJustify),
};

const LABEL_ELLIPSIZE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "ellipsize",
    value_shape: GtkPropertyValueShape::Enum(ELLIPSIZE_MODE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::LabelEllipsize),
};

const LABEL_MAX_WIDTH_CHARS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "maxWidthChars",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::LabelMaxWidthChars),
};

const LABEL_SELECTABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selectable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelSelectable),
};

const LABEL_USE_MARKUP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "useMarkup",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelUseMarkup),
};

const LABEL_LINES_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "lines",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::LabelLines),
};

const LABEL_XALIGN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "xalign",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::LabelXalign),
};

const LABEL_YALIGN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "yalign",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::LabelYalign),
};

const LABEL_WIDTH_CHARS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "widthChars",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::LabelWidthChars),
};

const LABEL_SINGLE_LINE_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "singleLineMode",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::LabelSingleLineMode),
};

// ── Entry-specific properties ────────────────────────────────────────────────

const INPUT_PURPOSE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "InputPurpose",
    variants: &[
        "FreeForm", "Alpha", "Digits", "Number", "Phone", "Url", "Email", "Name", "Password", "Pin",
    ],
};

const ENTRY_VISIBILITY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "visibility",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::EntryVisibility),
};

const ENTRY_MAX_LENGTH_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "maxLength",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::EntryMaxLength),
};

const ENTRY_INPUT_PURPOSE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "inputPurpose",
    value_shape: GtkPropertyValueShape::Enum(INPUT_PURPOSE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntryInputPurpose),
};

const ENTRY_PRIMARY_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "primaryIconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntryPrimaryIconName),
};

const ENTRY_SECONDARY_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "secondaryIconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntrySecondaryIconName),
};

// ── ScrolledWindow-specific properties ───────────────────────────────────────

const POLICY_TYPE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "PolicyType",
    variants: &["Always", "Automatic", "Never", "External"],
};

const SCROLLED_WINDOW_H_POLICY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "hscrollbarPolicy",
    value_shape: GtkPropertyValueShape::Enum(POLICY_TYPE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ScrolledWindowHPolicy),
};

const SCROLLED_WINDOW_V_POLICY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "vscrollbarPolicy",
    value_shape: GtkPropertyValueShape::Enum(POLICY_TYPE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ScrolledWindowVPolicy),
};

const SCROLLED_WINDOW_PROPAGATE_NATURAL_WIDTH_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "propagateNaturalWidth",
        value_shape: GtkPropertyValueShape::Bool,
        setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ScrolledWindowPropagateNaturalWidth),
    };

const SCROLLED_WINDOW_PROPAGATE_NATURAL_HEIGHT_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "propagateNaturalHeight",
        value_shape: GtkPropertyValueShape::Bool,
        setter: GtkPropertySetter::Bool(
            GtkBoolPropertySetter::ScrolledWindowPropagateNaturalHeight,
        ),
    };

// ── ProgressBar-specific properties ──────────────────────────────────────────

const PROGRESS_BAR_SHOW_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showText",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ProgressBarShowText),
};

// ── Image-specific properties ─────────────────────────────────────────────────

const IMAGE_FILE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "file",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ImageFile),
};

// ── Box-specific properties ───────────────────────────────────────────────────

const BOX_HOMOGENEOUS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "homogeneous",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::BoxHomogeneous),
};

// ── Revealer event ────────────────────────────────────────────────────────────

const REVEALER_CHILD_REVEALED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChildRevealed",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::RevealerChildRevealed,
};

// ── Focus events ──────────────────────────────────────────────────────────────

const FOCUS_IN_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onFocusIn",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::FocusIn,
};

const FOCUS_OUT_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onFocusOut",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::FocusOut,
};

// ── Scroll event ──────────────────────────────────────────────────────────────

const SCROLL_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onScroll",
    payload: GtkConcreteEventPayload::F64,
    signal: GtkEventSignal::Scroll,
};

// ── Pointer events ────────────────────────────────────────────────────────────

const POINTER_ENTER_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPointerEnter",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::PointerEnter,
};

const POINTER_LEAVE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPointerLeave",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::PointerLeave,
};

// ── Gesture events ────────────────────────────────────────────────────────────

const SECONDARY_CLICK_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSecondaryClick",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::SecondaryClick,
};

const LONG_PRESS_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onLongPress",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::LongPress,
};

const SWIPE_LEFT_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSwipeLeft",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::SwipeLeft,
};

const SWIPE_RIGHT_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSwipeRight",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::SwipeRight,
};

// ── Adwaita: StatusPage ───────────────────────────────────────────────────────

const STATUS_PAGE_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageTitle),
};

const STATUS_PAGE_DESCRIPTION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "description",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageDescription),
};

const STATUS_PAGE_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::StatusPageIconName),
};

const STATUS_PAGE_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::StatusPageContent,
    min_children: 0,
    max_children: Some(1),
};

// ── Adwaita: Clamp ────────────────────────────────────────────────────────────

const CLAMP_MAXIMUM_SIZE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "maximumSize",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::ClampMaximumSize),
};

const CLAMP_TIGHTENING_THRESHOLD_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "tighteningThreshold",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::ClampTighteningThreshold),
};

const CLAMP_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ClampContent,
    min_children: 0,
    max_children: Some(1),
};

// ── Adwaita: Banner ───────────────────────────────────────────────────────────

const BANNER_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::BannerTitle),
};

const BANNER_BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "buttonLabel",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::BannerButtonLabel),
};

const BANNER_REVEALED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "revealed",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::BannerRevealed),
};

const BANNER_BUTTON_CLICKED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onButtonClicked",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::BannerButtonClicked,
};

// ── Adwaita: ToolbarView ──────────────────────────────────────────────────────

const TOOLBAR_VIEW_TOP_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "topBar",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ToolbarViewTop,
    min_children: 0,
    max_children: Some(1),
};

const TOOLBAR_VIEW_BOTTOM_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "bottomBar",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ToolbarViewBottom,
    min_children: 0,
    max_children: Some(1),
};

const TOOLBAR_VIEW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ToolbarViewContent,
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
        FOCUSABLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        WINDOW_TITLE_PROPERTY,
        WINDOW_DEFAULT_WIDTH_PROPERTY,
        WINDOW_DEFAULT_HEIGHT_PROPERTY,
        WINDOW_RESIZABLE_PROPERTY,
        WINDOW_MODAL_PROPERTY,
        WINDOW_MAXIMIZED_PROPERTY,
        WINDOW_FULLSCREEN_PROPERTY,
        WINDOW_DECORATED_PROPERTY,
        WINDOW_HIDE_ON_CLOSE_PROPERTY,
    ],
    events: &[
        WINDOW_CLOSE_REQUEST_EVENT,
        WINDOW_MAXIMIZE_EVENT,
        WINDOW_FULLSCREEN_EVENT,
    ],
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
        FOCUSABLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        HEADER_BAR_SHOW_TITLE_BUTTONS_PROPERTY,
        HEADER_BAR_DECORATION_LAYOUT_PROPERTY,
        HEADER_BAR_CENTERING_POLICY_PROPERTY,
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        BOX_ORIENTATION_PROPERTY,
        BOX_SPACING_PROPERTY,
        BOX_HOMOGENEOUS_PROPERTY,
    ],
    events: &[
        SECONDARY_CLICK_EVENT,
        LONG_PRESS_EVENT,
        SWIPE_LEFT_EVENT,
        SWIPE_RIGHT_EVENT,
    ],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SCROLLED_WINDOW_H_POLICY_PROPERTY,
        SCROLLED_WINDOW_V_POLICY_PROPERTY,
        SCROLLED_WINDOW_PROPAGATE_NATURAL_WIDTH_PROPERTY,
        SCROLLED_WINDOW_PROPAGATE_NATURAL_HEIGHT_PROPERTY,
    ],
    events: &[SCROLL_EVENT],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        MONOSPACE_PROPERTY,
        LABEL_TEXT_PROPERTY,
        LABEL_LABEL_PROPERTY,
        LABEL_WRAP_PROPERTY,
        LABEL_WRAP_MODE_PROPERTY,
        LABEL_JUSTIFY_PROPERTY,
        LABEL_ELLIPSIZE_PROPERTY,
        LABEL_MAX_WIDTH_CHARS_PROPERTY,
        LABEL_SELECTABLE_PROPERTY,
        LABEL_USE_MARKUP_PROPERTY,
        LABEL_LINES_PROPERTY,
        LABEL_XALIGN_PROPERTY,
        LABEL_YALIGN_PROPERTY,
        LABEL_WIDTH_CHARS_PROPERTY,
        LABEL_SINGLE_LINE_MODE_PROPERTY,
    ],
    events: &[
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
        POINTER_ENTER_EVENT,
        POINTER_LEAVE_EVENT,
        SECONDARY_CLICK_EVENT,
        LONG_PRESS_EVENT,
        SWIPE_LEFT_EVENT,
        SWIPE_RIGHT_EVENT,
    ],
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
        FOCUSABLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        BUTTON_COMPACT_PROPERTY,
        BUTTON_HAS_FRAME_PROPERTY,
        BUTTON_LABEL_PROPERTY,
        BUTTON_ICON_NAME_PROPERTY,
        BUTTON_USE_UNDERLINE_PROPERTY,
        BUTTON_RECEIVES_DEFAULT_PROPERTY,
    ],
    events: &[
        BUTTON_CLICK_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
        POINTER_ENTER_EVENT,
        POINTER_LEAVE_EVENT,
        SECONDARY_CLICK_EVENT,
        LONG_PRESS_EVENT,
        SWIPE_LEFT_EVENT,
        SWIPE_RIGHT_EVENT,
    ],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ENTRY_TEXT_PROPERTY,
        ENTRY_PLACEHOLDER_TEXT_PROPERTY,
        ENTRY_EDITABLE_PROPERTY,
        ENTRY_VISIBILITY_PROPERTY,
        ENTRY_MAX_LENGTH_PROPERTY,
        ENTRY_INPUT_PURPOSE_PROPERTY,
        ENTRY_PRIMARY_ICON_NAME_PROPERTY,
        ENTRY_SECONDARY_ICON_NAME_PROPERTY,
    ],
    events: &[
        ENTRY_CHANGE_EVENT,
        ENTRY_ACTIVATE_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SWITCH_ACTIVE_PROPERTY,
    ],
    events: &[SWITCH_TOGGLE_EVENT, FOCUS_IN_EVENT, FOCUS_OUT_EVENT],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        CHECK_BUTTON_LABEL_PROPERTY,
        CHECK_BUTTON_ACTIVE_PROPERTY,
    ],
    events: &[CHECK_BUTTON_TOGGLE_EVENT, FOCUS_IN_EVENT, FOCUS_OUT_EVENT],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        TOGGLE_BUTTON_LABEL_PROPERTY,
        TOGGLE_BUTTON_ACTIVE_PROPERTY,
    ],
    events: &[TOGGLE_BUTTON_TOGGLE_EVENT, FOCUS_IN_EVENT, FOCUS_OUT_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── SpinButton ──────────────────────────────────────────────────────────────

const SPIN_BUTTON_VALUE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "value",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonValue),
};

const SPIN_BUTTON_MIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "min",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonMin),
};

const SPIN_BUTTON_MAX_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "max",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonMax),
};

const SPIN_BUTTON_STEP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "step",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinButtonStep),
};

const SPIN_BUTTON_DIGITS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "digits",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::SpinButtonDigits),
};

const SPIN_BUTTON_WRAP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "wrap",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinButtonWrap),
};

const SPIN_BUTTON_NUMERIC_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "numeric",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SpinButtonNumeric),
};

const SPIN_BUTTON_VALUE_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onValueChanged",
    payload: GtkConcreteEventPayload::F64,
    signal: GtkEventSignal::SpinButtonValueChanged,
};

const SPIN_BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "SpinButton",
    kind: GtkConcreteWidgetKind::SpinButton,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SPIN_BUTTON_VALUE_PROPERTY,
        SPIN_BUTTON_MIN_PROPERTY,
        SPIN_BUTTON_MAX_PROPERTY,
        SPIN_BUTTON_STEP_PROPERTY,
        SPIN_BUTTON_DIGITS_PROPERTY,
        SPIN_BUTTON_WRAP_PROPERTY,
        SPIN_BUTTON_NUMERIC_PROPERTY,
    ],
    events: &[
        SPIN_BUTTON_VALUE_CHANGED_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Scale ───────────────────────────────────────────────────────────────────

const SCALE_VALUE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "value",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ScaleValue),
};

const SCALE_MIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "min",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ScaleMin),
};

const SCALE_MAX_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "max",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ScaleMax),
};

const SCALE_STEP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "step",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ScaleStep),
};

const SCALE_DIGITS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "digits",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::ScaleDigits),
};

const SCALE_DRAW_VALUE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "drawValue",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ScaleDrawValue),
};

const SCALE_ORIENTATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "orientation",
    value_shape: GtkPropertyValueShape::Enum(ORIENTATION_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ScaleOrientation),
};

const SCALE_VALUE_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onValueChanged",
    payload: GtkConcreteEventPayload::F64,
    signal: GtkEventSignal::ScaleValueChanged,
};

// ── Scale extra properties ───────────────────────────────────────────────────

const POSITION_TYPE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "PositionType",
    variants: &["Top", "Bottom", "Left", "Right"],
};

const SCALE_VALUE_POS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "valuePos",
    value_shape: GtkPropertyValueShape::Enum(POSITION_TYPE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ScaleValuePos),
};

const SCALE_FILL_LEVEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "fillLevel",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::ScaleFillLevel),
};

const SCALE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Scale",
    kind: GtkConcreteWidgetKind::Scale,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SCALE_VALUE_PROPERTY,
        SCALE_MIN_PROPERTY,
        SCALE_MAX_PROPERTY,
        SCALE_STEP_PROPERTY,
        SCALE_DIGITS_PROPERTY,
        SCALE_DRAW_VALUE_PROPERTY,
        SCALE_ORIENTATION_PROPERTY,
        SCALE_VALUE_POS_PROPERTY,
        SCALE_FILL_LEVEL_PROPERTY,
    ],
    events: &[SCALE_VALUE_CHANGED_EVENT, FOCUS_IN_EVENT, FOCUS_OUT_EVENT],
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
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        IMAGE_ICON_NAME_PROPERTY,
        IMAGE_RESOURCE_PATH_PROPERTY,
        IMAGE_PIXEL_SIZE_PROPERTY,
        IMAGE_FILE_PROPERTY,
    ],
    events: &[
        SECONDARY_CLICK_EVENT,
        LONG_PRESS_EVENT,
        SWIPE_LEFT_EVENT,
        SWIPE_RIGHT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Picture-specific properties ───────────────────────────────────────────────

const PICTURE_FILENAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "filename",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PictureFilename),
};

const PICTURE_RESOURCE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "resource",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PictureResource),
};

const PICTURE_CONTENT_FIT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "contentFit",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PictureContentFit),
};

const PICTURE_ALT_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "altText",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PictureAltText),
};

const PICTURE_CAN_SHRINK_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "canShrink",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::PictureCanShrink),
};

const WEB_VIEW_HTML_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "html",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::WebViewHtml),
};

const PICTURE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Picture",
    kind: GtkConcreteWidgetKind::Picture,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        PICTURE_FILENAME_PROPERTY,
        PICTURE_RESOURCE_PROPERTY,
        PICTURE_CONTENT_FIT_PROPERTY,
        PICTURE_ALT_TEXT_PROPERTY,
        PICTURE_CAN_SHRINK_PROPERTY,
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
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
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
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        PROGRESS_BAR_FRACTION_PROPERTY,
        PROGRESS_BAR_TEXT_PROPERTY,
        PROGRESS_BAR_SHOW_TEXT_PROPERTY,
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
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        REVEALER_REVEALED_PROPERTY,
        REVEALER_TRANSITION_TYPE_PROPERTY,
        REVEALER_TRANSITION_DURATION_PROPERTY,
    ],
    events: &[REVEALER_CHILD_REVEALED_EVENT],
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
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SEPARATOR_ORIENTATION_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

const STATUS_PAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "StatusPage",
    kind: GtkConcreteWidgetKind::StatusPage,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        STATUS_PAGE_TITLE_PROPERTY,
        STATUS_PAGE_DESCRIPTION_PROPERTY,
        STATUS_PAGE_ICON_NAME_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[STATUS_PAGE_CONTENT_CHILD_GROUP],
};

const CLAMP_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Clamp",
    kind: GtkConcreteWidgetKind::Clamp,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        CLAMP_MAXIMUM_SIZE_PROPERTY,
        CLAMP_TIGHTENING_THRESHOLD_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[CLAMP_CONTENT_CHILD_GROUP],
};

const BANNER_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Banner",
    kind: GtkConcreteWidgetKind::Banner,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        BANNER_TITLE_PROPERTY,
        BANNER_BUTTON_LABEL_PROPERTY,
        BANNER_REVEALED_PROPERTY,
    ],
    events: &[BANNER_BUTTON_CLICKED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

const TOOLBAR_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ToolbarView",
    kind: GtkConcreteWidgetKind::ToolbarView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&TOOLBAR_VIEW_CONTENT_CHILD_GROUP),
    child_groups: &[
        TOOLBAR_VIEW_TOP_CHILD_GROUP,
        TOOLBAR_VIEW_BOTTOM_CHILD_GROUP,
        TOOLBAR_VIEW_CONTENT_CHILD_GROUP,
    ],
};

// ── Adwaita preference row shared properties ──────────────────────────────────

const ADW_PREFERENCES_ROW_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AdwPreferencesRowTitle),
};

const ADW_ACTION_ROW_SUBTITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "subtitle",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AdwActionRowSubtitle),
};

const ADW_EXPANDER_ROW_SUBTITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "subtitle",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AdwExpanderRowSubtitle),
};

const LIST_BOX_ROW_ACTIVATABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "activatable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ListBoxRowActivatable),
};

// ── Adwaita: ActionRow ────────────────────────────────────────────────────────

const ACTION_ROW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::ActionRowActivated,
};

const ACTION_ROW_SUFFIX_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "suffix",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ActionRowSuffix,
    min_children: 0,
    max_children: None,
};

const ACTION_ROW_PREFIX_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "prefix",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ActionRowPrefix,
    min_children: 0,
    max_children: None,
};

const ACTION_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ActionRow",
    kind: GtkConcreteWidgetKind::ActionRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ADW_ACTION_ROW_SUBTITLE_PROPERTY,
        LIST_BOX_ROW_ACTIVATABLE_PROPERTY,
    ],
    events: &[ACTION_ROW_ACTIVATED_EVENT],
    default_child_group_override: None,
    child_groups: &[ACTION_ROW_PREFIX_CHILD_GROUP, ACTION_ROW_SUFFIX_CHILD_GROUP],
};

// ── Adwaita: ExpanderRow ──────────────────────────────────────────────────────

const EXPANDER_ROW_EXPANDED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "expanded",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ExpanderRowExpanded),
};

const EXPANDER_ROW_EXPANDED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onExpanded",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::ExpanderRowExpanded,
};

const EXPANDER_ROW_ROWS_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "rows",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ExpanderRowRows,
    min_children: 0,
    max_children: None,
};

const EXPANDER_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ExpanderRow",
    kind: GtkConcreteWidgetKind::ExpanderRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ADW_EXPANDER_ROW_SUBTITLE_PROPERTY,
        EXPANDER_ROW_EXPANDED_PROPERTY,
    ],
    events: &[EXPANDER_ROW_EXPANDED_EVENT],
    default_child_group_override: None,
    child_groups: &[EXPANDER_ROW_ROWS_CHILD_GROUP],
};

// ── Adwaita: SwitchRow ────────────────────────────────────────────────────────

const SWITCH_ROW_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::SwitchRowActive),
};

const SWITCH_ROW_TOGGLED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onToggled",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::SwitchRowToggled,
};

const SWITCH_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "SwitchRow",
    kind: GtkConcreteWidgetKind::SwitchRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ADW_ACTION_ROW_SUBTITLE_PROPERTY,
        SWITCH_ROW_ACTIVE_PROPERTY,
    ],
    events: &[SWITCH_ROW_TOGGLED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Adwaita: SpinRow ──────────────────────────────────────────────────────────

const SPIN_ROW_VALUE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "value",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowValue),
};

const SPIN_ROW_MIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "min",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowMin),
};

const SPIN_ROW_MAX_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "max",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowMax),
};

const SPIN_ROW_STEP_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "step",
    value_shape: GtkPropertyValueShape::F64,
    setter: GtkPropertySetter::F64(GtkF64PropertySetter::SpinRowStep),
};

const SPIN_ROW_VALUE_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onValueChanged",
    payload: GtkConcreteEventPayload::F64,
    signal: GtkEventSignal::SpinRowValueChanged,
};

const SPIN_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "SpinRow",
    kind: GtkConcreteWidgetKind::SpinRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ADW_ACTION_ROW_SUBTITLE_PROPERTY,
        SPIN_ROW_VALUE_PROPERTY,
        SPIN_ROW_MIN_PROPERTY,
        SPIN_ROW_MAX_PROPERTY,
        SPIN_ROW_STEP_PROPERTY,
    ],
    events: &[SPIN_ROW_VALUE_CHANGED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Adwaita: EntryRow ─────────────────────────────────────────────────────────

const ENTRY_ROW_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::EntryRowText),
};

const ENTRY_ROW_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChange",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::EntryRowChanged,
};

const ENTRY_ROW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::EntryRowActivated,
};

const ENTRY_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "EntryRow",
    kind: GtkConcreteWidgetKind::EntryRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ENTRY_ROW_TEXT_PROPERTY,
    ],
    events: &[
        ENTRY_ROW_CHANGED_EVENT,
        ENTRY_ROW_ACTIVATED_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

// ── ListBox ───────────────────────────────────────────────────────────────────

const SELECTION_MODE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "SelectionMode",
    variants: &["None", "Single", "Browse", "Multiple"],
};

const LIST_BOX_SELECTION_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selectionMode",
    value_shape: GtkPropertyValueShape::Enum(SELECTION_MODE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ListBoxSelectionMode),
};

const LIST_BOX_SHOW_SEPARATORS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showSeparators",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ListBoxShowSeparators),
};

const LIST_BOX_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onRowActivated",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::ListBoxActivated,
};

const LIST_BOX_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ListBoxChildren,
    min_children: 0,
    max_children: None,
};

const LIST_BOX_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ListBox",
    kind: GtkConcreteWidgetKind::ListBox,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        LIST_BOX_SELECTION_MODE_PROPERTY,
        LIST_BOX_SHOW_SEPARATORS_PROPERTY,
    ],
    events: &[LIST_BOX_ACTIVATED_EVENT],
    default_child_group_override: None,
    child_groups: &[LIST_BOX_CHILDREN_CHILD_GROUP],
};

// ── ListBoxRow ────────────────────────────────────────────────────────────────

const LIST_BOX_ROW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::ListBoxRowActivated,
};

const LIST_BOX_ROW_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "child",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ListBoxRowChild,
    min_children: 0,
    max_children: Some(1),
};

const LIST_BOX_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ListBoxRow",
    kind: GtkConcreteWidgetKind::ListBoxRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        LIST_BOX_ROW_ACTIVATABLE_PROPERTY,
    ],
    events: &[LIST_BOX_ROW_ACTIVATED_EVENT],
    default_child_group_override: None,
    child_groups: &[LIST_BOX_ROW_CHILD_GROUP],
};

// ── ListView ──────────────────────────────────────────────────────────────────

const LIST_VIEW_SHOW_SEPARATORS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showSeparators",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewShowSeparators),
};

const LIST_VIEW_ENABLE_RUBBERBAND_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "enableRubberband",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewEnableRubberband),
};

const LIST_VIEW_SINGLE_CLICK_ACTIVATE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "singleClickActivate",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ListViewSingleClickActivate),
};

const LIST_VIEW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivate",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::ListViewActivated,
};

const LIST_VIEW_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ListViewChildren,
    min_children: 0,
    max_children: None,
};

const LIST_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ListView",
    kind: GtkConcreteWidgetKind::ListView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        LIST_VIEW_SHOW_SEPARATORS_PROPERTY,
        LIST_VIEW_ENABLE_RUBBERBAND_PROPERTY,
        LIST_VIEW_SINGLE_CLICK_ACTIVATE_PROPERTY,
    ],
    events: &[LIST_VIEW_ACTIVATED_EVENT],
    default_child_group_override: Some(&LIST_VIEW_CHILDREN_CHILD_GROUP),
    child_groups: &[LIST_VIEW_CHILDREN_CHILD_GROUP],
};

// ── GridView ──────────────────────────────────────────────────────────────────

const GRID_VIEW_ENABLE_RUBBERBAND_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "enableRubberband",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::GridViewEnableRubberband),
};

const GRID_VIEW_SINGLE_CLICK_ACTIVATE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "singleClickActivate",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::GridViewSingleClickActivate),
};

const GRID_VIEW_MIN_COLUMNS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "minColumns",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridViewMinColumns),
};

const GRID_VIEW_MAX_COLUMNS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "maxColumns",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridViewMaxColumns),
};

const GRID_VIEW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivate",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::GridViewActivated,
};

const GRID_VIEW_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::GridViewChildren,
    min_children: 0,
    max_children: None,
};

const GRID_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "GridView",
    kind: GtkConcreteWidgetKind::GridView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        GRID_VIEW_ENABLE_RUBBERBAND_PROPERTY,
        GRID_VIEW_SINGLE_CLICK_ACTIVATE_PROPERTY,
        GRID_VIEW_MIN_COLUMNS_PROPERTY,
        GRID_VIEW_MAX_COLUMNS_PROPERTY,
    ],
    events: &[GRID_VIEW_ACTIVATED_EVENT],
    default_child_group_override: Some(&GRID_VIEW_CHILDREN_CHILD_GROUP),
    child_groups: &[GRID_VIEW_CHILDREN_CHILD_GROUP],
};

// ── DropDown ──────────────────────────────────────────────────────────────────

const DROP_DOWN_ITEMS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "items",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::DropDownItems),
};

const DROP_DOWN_SELECTED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selected",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::DropDownSelected),
};

const DROP_DOWN_SELECTION_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSelectionChanged",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::DropDownSelectionChanged,
};

const DROP_DOWN_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "DropDown",
    kind: GtkConcreteWidgetKind::DropDown,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        DROP_DOWN_ITEMS_PROPERTY,
        DROP_DOWN_SELECTED_PROPERTY,
    ],
    events: &[DROP_DOWN_SELECTION_CHANGED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── SearchEntry ───────────────────────────────────────────────────────────────

const SEARCH_ENTRY_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::SearchEntryText),
};

const SEARCH_ENTRY_PLACEHOLDER_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "placeholder",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::SearchEntryPlaceholder),
};

const SEARCH_ENTRY_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChange",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::SearchEntryChanged,
};

const SEARCH_ENTRY_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::SearchEntryActivated,
};

const SEARCH_ENTRY_SEARCH_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSearchChanged",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::SearchEntrySearchChanged,
};

const SEARCH_ENTRY_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "SearchEntry",
    kind: GtkConcreteWidgetKind::SearchEntry,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SEARCH_ENTRY_TEXT_PROPERTY,
        SEARCH_ENTRY_PLACEHOLDER_PROPERTY,
    ],
    events: &[
        SEARCH_ENTRY_CHANGED_EVENT,
        SEARCH_ENTRY_ACTIVATED_EVENT,
        SEARCH_ENTRY_SEARCH_CHANGED_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Expander ──────────────────────────────────────────────────────────────────

const EXPANDER_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ExpanderLabel),
};

const EXPANDER_EXPANDED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "expanded",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::ExpanderExpanded),
};

const EXPANDER_EXPANDED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onExpanded",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::ExpanderExpanded,
};

const EXPANDER_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "child",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ExpanderChild,
    min_children: 0,
    max_children: Some(1),
};

const EXPANDER_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Expander",
    kind: GtkConcreteWidgetKind::Expander,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        EXPANDER_LABEL_PROPERTY,
        EXPANDER_EXPANDED_PROPERTY,
    ],
    events: &[EXPANDER_EXPANDED_EVENT],
    default_child_group_override: None,
    child_groups: &[EXPANDER_CHILD_GROUP],
};

// ── Adwaita: NavigationView ───────────────────────────────────────────────────

const NAVIGATION_VIEW_PAGES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "pages",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::NavigationViewPages,
    min_children: 0,
    max_children: None,
};

const NAVIGATION_VIEW_POPPED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPopped",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::NavigationViewPopped,
};

const NAVIGATION_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "NavigationView",
    kind: GtkConcreteWidgetKind::NavigationView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[NAVIGATION_VIEW_POPPED_EVENT],
    default_child_group_override: None,
    child_groups: &[NAVIGATION_VIEW_PAGES_CHILD_GROUP],
};

// ── Adwaita: NavigationPage ───────────────────────────────────────────────────

const NAVIGATION_PAGE_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::NavigationPageTitle),
};

const NAVIGATION_PAGE_TAG_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "tag",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::NavigationPageTag),
};

const NAVIGATION_PAGE_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::NavigationPageContent,
    min_children: 0,
    max_children: Some(1),
};

const NAVIGATION_PAGE_SHOWING_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onShowing",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::NavigationPageShowing,
};

const NAVIGATION_PAGE_HIDING_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onHiding",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::NavigationPageHiding,
};

const NAVIGATION_PAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "NavigationPage",
    kind: GtkConcreteWidgetKind::NavigationPage,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        NAVIGATION_PAGE_TITLE_PROPERTY,
        NAVIGATION_PAGE_TAG_PROPERTY,
    ],
    events: &[NAVIGATION_PAGE_SHOWING_EVENT, NAVIGATION_PAGE_HIDING_EVENT],
    default_child_group_override: None,
    child_groups: &[NAVIGATION_PAGE_CONTENT_CHILD_GROUP],
};

// ── Adwaita: ToastOverlay ─────────────────────────────────────────────────────

const TOAST_OVERLAY_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ToastOverlayContent,
    min_children: 0,
    max_children: Some(1),
};

const TOAST_OVERLAY_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ToastOverlay",
    kind: GtkConcreteWidgetKind::ToastOverlay,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[TOAST_OVERLAY_CONTENT_CHILD_GROUP],
};

// ── Adwaita: PreferencesGroup ─────────────────────────────────────────────────

const PREFERENCES_GROUP_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesGroupTitle),
};

const PREFERENCES_GROUP_DESCRIPTION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "description",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesGroupDescription),
};

const PREFERENCES_GROUP_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::PreferencesGroupChildren,
    min_children: 0,
    max_children: None,
};

const PREFERENCES_GROUP_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "PreferencesGroup",
    kind: GtkConcreteWidgetKind::PreferencesGroup,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        PREFERENCES_GROUP_TITLE_PROPERTY,
        PREFERENCES_GROUP_DESCRIPTION_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[PREFERENCES_GROUP_CHILDREN_CHILD_GROUP],
};

// ── Adwaita: PreferencesPage ──────────────────────────────────────────────────

const PREFERENCES_PAGE_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesPageTitle),
};

const PREFERENCES_PAGE_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PreferencesPageIconName),
};

const PREFERENCES_PAGE_GROUPS_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::PreferencesPageChildren,
    min_children: 0,
    max_children: None,
};

const PREFERENCES_PAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "PreferencesPage",
    kind: GtkConcreteWidgetKind::PreferencesPage,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        PREFERENCES_PAGE_TITLE_PROPERTY,
        PREFERENCES_PAGE_ICON_NAME_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[PREFERENCES_PAGE_GROUPS_CHILD_GROUP],
};

// ── Adwaita: PreferencesWindow ────────────────────────────────────────────────

const PREFERENCES_WINDOW_SEARCH_ENABLED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "searchEnabled",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::PreferencesWindowSearchEnabled),
};

const PREFERENCES_WINDOW_PAGES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "pages",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::PreferencesWindowPages,
    min_children: 0,
    max_children: None,
};

const PREFERENCES_WINDOW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "PreferencesWindow",
    kind: GtkConcreteWidgetKind::PreferencesWindow,
    root_kind: GtkWidgetRootKind::Window,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        FOCUSABLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        WINDOW_TITLE_PROPERTY,
        WINDOW_DEFAULT_WIDTH_PROPERTY,
        WINDOW_DEFAULT_HEIGHT_PROPERTY,
        WINDOW_RESIZABLE_PROPERTY,
        WINDOW_MODAL_PROPERTY,
        PREFERENCES_WINDOW_SEARCH_ENABLED_PROPERTY,
    ],
    events: &[WINDOW_CLOSE_REQUEST_EVENT],
    default_child_group_override: Some(&PREFERENCES_WINDOW_PAGES_CHILD_GROUP),
    child_groups: &[PREFERENCES_WINDOW_PAGES_CHILD_GROUP],
};

// ── Adwaita: ComboRow ─────────────────────────────────────────────────────────

const COMBO_ROW_ITEMS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "items",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ComboRowItems),
};

const COMBO_ROW_SELECTED_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selected",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::ComboRowSelected),
};

const COMBO_ROW_SELECTION_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSelectionChanged",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::ComboRowSelectionChanged,
};

const COMBO_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ComboRow",
    kind: GtkConcreteWidgetKind::ComboRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        ADW_ACTION_ROW_SUBTITLE_PROPERTY,
        COMBO_ROW_ITEMS_PROPERTY,
        COMBO_ROW_SELECTED_PROPERTY,
    ],
    events: &[COMBO_ROW_SELECTION_CHANGED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Adwaita: PasswordEntryRow ─────────────────────────────────────────────────

const PASSWORD_ENTRY_ROW_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::PasswordEntryRowText),
};

const PASSWORD_ENTRY_ROW_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChange",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::PasswordEntryRowChanged,
};

const PASSWORD_ENTRY_ROW_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::PasswordEntryRowActivated,
};

const PASSWORD_ENTRY_ROW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "PasswordEntryRow",
    kind: GtkConcreteWidgetKind::PasswordEntryRow,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        ADW_PREFERENCES_ROW_TITLE_PROPERTY,
        PASSWORD_ENTRY_ROW_TEXT_PROPERTY,
    ],
    events: &[
        PASSWORD_ENTRY_ROW_CHANGED_EVENT,
        PASSWORD_ENTRY_ROW_ACTIVATED_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

// ── GTK: Overlay ──────────────────────────────────────────────────────────────

const OVERLAY_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::OverlayContent,
    min_children: 0,
    max_children: Some(1),
};

const OVERLAY_OVERLAY_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "overlay",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::OverlayOverlay,
    min_children: 0,
    max_children: None,
};

const OVERLAY_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Overlay",
    kind: GtkConcreteWidgetKind::Overlay,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&OVERLAY_CONTENT_CHILD_GROUP),
    child_groups: &[OVERLAY_CONTENT_CHILD_GROUP, OVERLAY_OVERLAY_CHILD_GROUP],
};

// ── GTK: MultilineEntry ───────────────────────────────────────────────────────

const MULTILINE_ENTRY_TEXT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "text",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::MultilineEntryText),
};

const MULTILINE_ENTRY_EDITABLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "editable",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::MultilineEntryEditable),
};

const MULTILINE_ENTRY_WRAP_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "wrapMode",
    value_shape: GtkPropertyValueShape::Enum(WRAP_MODE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::MultilineEntryWrapMode),
};

const MULTILINE_ENTRY_MONOSPACE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "monospace",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::MultilineEntryMonospace),
};

const MULTILINE_ENTRY_TOP_MARGIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "topMargin",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryTopMargin),
};

const MULTILINE_ENTRY_BOTTOM_MARGIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "bottomMargin",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryBottomMargin),
};

const MULTILINE_ENTRY_LEFT_MARGIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "leftMargin",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryLeftMargin),
};

const MULTILINE_ENTRY_RIGHT_MARGIN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "rightMargin",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::MultilineEntryRightMargin),
};

const MULTILINE_ENTRY_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChange",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::MultilineEntryChanged,
};

const MULTILINE_ENTRY_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "MultilineEntry",
    kind: GtkConcreteWidgetKind::MultilineEntry,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        MULTILINE_ENTRY_TEXT_PROPERTY,
        MULTILINE_ENTRY_EDITABLE_PROPERTY,
        MULTILINE_ENTRY_WRAP_MODE_PROPERTY,
        MULTILINE_ENTRY_MONOSPACE_PROPERTY,
        MULTILINE_ENTRY_TOP_MARGIN_PROPERTY,
        MULTILINE_ENTRY_BOTTOM_MARGIN_PROPERTY,
        MULTILINE_ENTRY_LEFT_MARGIN_PROPERTY,
        MULTILINE_ENTRY_RIGHT_MARGIN_PROPERTY,
    ],
    events: &[
        MULTILINE_ENTRY_CHANGED_EVENT,
        FOCUS_IN_EVENT,
        FOCUS_OUT_EVENT,
    ],
    default_child_group_override: None,
    child_groups: &[],
};

const WEB_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "WebView",
    kind: GtkConcreteWidgetKind::WebView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        WEB_VIEW_HTML_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Adwaita: ViewStack ────────────────────────────────────────────────────────

const VIEW_STACK_VISIBLE_CHILD_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "visibleChildName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackVisibleChild),
};

const VIEW_STACK_SWITCH_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSwitch",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::ViewStackSwitch,
};

const VIEW_STACK_PAGES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "pages",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::ViewStackPages,
    min_children: 0,
    max_children: None,
};

const VIEW_STACK_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ViewStack",
    kind: GtkConcreteWidgetKind::ViewStack,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        VIEW_STACK_VISIBLE_CHILD_PROPERTY,
    ],
    events: &[VIEW_STACK_SWITCH_EVENT],
    default_child_group_override: Some(&VIEW_STACK_PAGES_CHILD_GROUP),
    child_groups: &[VIEW_STACK_PAGES_CHILD_GROUP],
};

// ── Adwaita: ViewStackPage ────────────────────────────────────────────────────

const VIEW_STACK_PAGE_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "name",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageName),
};

const VIEW_STACK_PAGE_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageTitle),
};

const VIEW_STACK_PAGE_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::ViewStackPageIconName),
};

const VIEW_STACK_PAGE_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::ViewStackPageContent,
    min_children: 0,
    max_children: Some(1),
};

const VIEW_STACK_PAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "ViewStackPage",
    kind: GtkConcreteWidgetKind::ViewStackPage,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        VIEW_STACK_PAGE_NAME_PROPERTY,
        VIEW_STACK_PAGE_TITLE_PROPERTY,
        VIEW_STACK_PAGE_ICON_NAME_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&VIEW_STACK_PAGE_CONTENT_CHILD_GROUP),
    child_groups: &[VIEW_STACK_PAGE_CONTENT_CHILD_GROUP],
};

// ── Calendar ─────────────────────────────────────────────────────────────────

const CALENDAR_YEAR_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "year",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::CalendarYear),
};

const CALENDAR_MONTH_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "month",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::CalendarMonth),
};

const CALENDAR_DAY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "day",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::CalendarDay),
};

const CALENDAR_DAY_SELECTED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onDaySelected",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::CalendarDaySelected,
};

const CALENDAR_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Calendar",
    kind: GtkConcreteWidgetKind::Calendar,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        CALENDAR_YEAR_PROPERTY,
        CALENDAR_MONTH_PROPERTY,
        CALENDAR_DAY_PROPERTY,
    ],
    events: &[CALENDAR_DAY_SELECTED_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── FlowBox ──────────────────────────────────────────────────────────────────

const FLOW_BOX_SELECTION_MODE_VALUE_SHAPE: GtkEnumValueShape = GtkEnumValueShape {
    name: "SelectionMode",
    variants: &["None", "Single", "Browse", "Multiple"],
};

const FLOW_BOX_SELECTION_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selectionMode",
    value_shape: GtkPropertyValueShape::Enum(FLOW_BOX_SELECTION_MODE_VALUE_SHAPE),
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FlowBoxSelectionMode),
};

const FLOW_BOX_ROW_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "rowSpacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::FlowBoxRowSpacing),
};

const FLOW_BOX_COLUMN_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "columnSpacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::FlowBoxColumnSpacing),
};

const FLOW_BOX_CHILD_ACTIVATED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onChildActivated",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::FlowBoxChildActivated,
};

const FLOW_BOX_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::FlowBoxChildren,
    min_children: 0,
    max_children: None,
};

const FLOW_BOX_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "FlowBox",
    kind: GtkConcreteWidgetKind::FlowBox,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        FLOW_BOX_SELECTION_MODE_PROPERTY,
        FLOW_BOX_ROW_SPACING_PROPERTY,
        FLOW_BOX_COLUMN_SPACING_PROPERTY,
    ],
    events: &[FLOW_BOX_CHILD_ACTIVATED_EVENT],
    default_child_group_override: Some(&FLOW_BOX_CHILDREN_CHILD_GROUP),
    child_groups: &[FLOW_BOX_CHILDREN_CHILD_GROUP],
};

// ── FlowBoxChild ─────────────────────────────────────────────────────────────

const FLOW_BOX_CHILD_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::FlowBoxChildContent,
    min_children: 0,
    max_children: Some(1),
};

const FLOW_BOX_CHILD_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "FlowBoxChild",
    kind: GtkConcreteWidgetKind::FlowBoxChild,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&FLOW_BOX_CHILD_CONTENT_CHILD_GROUP),
    child_groups: &[FLOW_BOX_CHILD_CONTENT_CHILD_GROUP],
};

// ── MenuButton ────────────────────────────────────────────────────────────────

const MENU_BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::MenuButtonLabel),
};

const MENU_BUTTON_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::MenuButtonIconName),
};

const MENU_BUTTON_ACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "active",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::MenuButtonActive),
};

const MENU_BUTTON_USE_UNDERLINE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "useUnderline",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::MenuButtonUseUnderline),
};

const MENU_BUTTON_TOGGLED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onToggled",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::MenuButtonToggled,
};

const MENU_BUTTON_POPOVER_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "popover",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::MenuButtonPopover,
    min_children: 0,
    max_children: Some(1),
};

const MENU_BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "MenuButton",
    kind: GtkConcreteWidgetKind::MenuButton,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        FOCUSABLE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        MENU_BUTTON_LABEL_PROPERTY,
        MENU_BUTTON_ICON_NAME_PROPERTY,
        MENU_BUTTON_ACTIVE_PROPERTY,
        MENU_BUTTON_USE_UNDERLINE_PROPERTY,
    ],
    events: &[MENU_BUTTON_TOGGLED_EVENT],
    default_child_group_override: Some(&MENU_BUTTON_POPOVER_CHILD_GROUP),
    child_groups: &[MENU_BUTTON_POPOVER_CHILD_GROUP],
};

// ── Popover ───────────────────────────────────────────────────────────────────

const POPOVER_AUTOHIDE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "autohide",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::PopoverAutohide),
};

const POPOVER_HAS_ARROW_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "hasArrow",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::PopoverHasArrow),
};

const POPOVER_CLOSED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onClosed",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::PopoverClosed,
};

const POPOVER_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::PopoverContent,
    min_children: 0,
    max_children: Some(1),
};

const POPOVER_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Popover",
    kind: GtkConcreteWidgetKind::Popover,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        POPOVER_AUTOHIDE_PROPERTY,
        POPOVER_HAS_ARROW_PROPERTY,
    ],
    events: &[POPOVER_CLOSED_EVENT],
    default_child_group_override: Some(&POPOVER_CONTENT_CHILD_GROUP),
    child_groups: &[POPOVER_CONTENT_CHILD_GROUP],
};

// ── Adwaita: AlertDialog (adw::MessageDialog) ─────────────────────────────────

const ALERT_DIALOG_HEADING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "heading",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogHeading),
};

const ALERT_DIALOG_BODY_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "body",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogBody),
};

const ALERT_DIALOG_DEFAULT_RESPONSE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "defaultResponse",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogDefaultResponse),
};

const ALERT_DIALOG_CLOSE_RESPONSE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "closeResponse",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogCloseResponse),
};

const ALERT_DIALOG_RESPONSES_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "responses",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AlertDialogResponses),
};

const ALERT_DIALOG_RESPONSE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onResponse",
    payload: GtkConcreteEventPayload::Text,
    signal: GtkEventSignal::AlertDialogResponse,
};

const ALERT_DIALOG_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "AlertDialog",
    kind: GtkConcreteWidgetKind::AlertDialog,
    root_kind: GtkWidgetRootKind::Window,
    properties: &[
        VISIBLE_PROPERTY,
        ALERT_DIALOG_HEADING_PROPERTY,
        ALERT_DIALOG_BODY_PROPERTY,
        ALERT_DIALOG_DEFAULT_RESPONSE_PROPERTY,
        ALERT_DIALOG_CLOSE_RESPONSE_PROPERTY,
        ALERT_DIALOG_RESPONSES_PROPERTY,
    ],
    events: &[ALERT_DIALOG_RESPONSE_EVENT],
    default_child_group_override: None,
    child_groups: &[],
};

// ── CenterBox ─────────────────────────────────────────────────────────────────

const CENTER_BOX_START_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "start",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::CenterBoxStart,
    min_children: 0,
    max_children: Some(1),
};

const CENTER_BOX_CENTER_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "center",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::CenterBoxCenter,
    min_children: 0,
    max_children: Some(1),
};

const CENTER_BOX_END_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "end",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::CenterBoxEnd,
    min_children: 0,
    max_children: Some(1),
};

const CENTER_BOX_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "CenterBox",
    kind: GtkConcreteWidgetKind::CenterBox,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&CENTER_BOX_CENTER_CHILD_GROUP),
    child_groups: &[
        CENTER_BOX_START_CHILD_GROUP,
        CENTER_BOX_CENTER_CHILD_GROUP,
        CENTER_BOX_END_CHILD_GROUP,
    ],
};

// ── AboutDialog ───────────────────────────────────────────────────────────────

const ABOUT_DIALOG_VISIBLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "visible",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::AboutDialogVisible),
};

const ABOUT_DIALOG_APP_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "appName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogAppName),
};

const ABOUT_DIALOG_VERSION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "version",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogVersion),
};

const ABOUT_DIALOG_DEVELOPER_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "developerName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogDeveloperName),
};

const ABOUT_DIALOG_COMMENTS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "comments",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogComments),
};

const ABOUT_DIALOG_WEBSITE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "website",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogWebsite),
};

const ABOUT_DIALOG_ISSUE_URL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "issueUrl",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogIssueUrl),
};

const ABOUT_DIALOG_LICENSE_TYPE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "licenseType",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogLicenseType),
};

const ABOUT_DIALOG_APPLICATION_ICON_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "applicationIcon",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::AboutDialogApplicationIcon),
};

const ABOUT_DIALOG_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "AboutDialog",
    kind: GtkConcreteWidgetKind::AboutDialog,
    root_kind: GtkWidgetRootKind::Window,
    properties: &[
        ABOUT_DIALOG_VISIBLE_PROPERTY,
        ABOUT_DIALOG_APP_NAME_PROPERTY,
        ABOUT_DIALOG_VERSION_PROPERTY,
        ABOUT_DIALOG_DEVELOPER_NAME_PROPERTY,
        ABOUT_DIALOG_COMMENTS_PROPERTY,
        ABOUT_DIALOG_WEBSITE_PROPERTY,
        ABOUT_DIALOG_ISSUE_URL_PROPERTY,
        ABOUT_DIALOG_LICENSE_TYPE_PROPERTY,
        ABOUT_DIALOG_APPLICATION_ICON_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

// ── SplitButton ───────────────────────────────────────────────────────────────

const SPLIT_BUTTON_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "label",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::SplitButtonLabel),
};

const SPLIT_BUTTON_ICON_NAME_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "iconName",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::SplitButtonIconName),
};

const SPLIT_BUTTON_CLICK_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onClick",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::SplitButtonClicked,
};

const SPLIT_BUTTON_POPOVER_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "popover",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::SplitButtonPopover,
    min_children: 0,
    max_children: Some(1),
};

const SPLIT_BUTTON_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "SplitButton",
    kind: GtkConcreteWidgetKind::SplitButton,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        SPLIT_BUTTON_LABEL_PROPERTY,
        SPLIT_BUTTON_ICON_NAME_PROPERTY,
    ],
    events: &[SPLIT_BUTTON_CLICK_EVENT],
    default_child_group_override: Some(&SPLIT_BUTTON_POPOVER_CHILD_GROUP),
    child_groups: &[SPLIT_BUTTON_POPOVER_CHILD_GROUP],
};

// ── NavigationSplitView ───────────────────────────────────────────────────────

const NAVIGATION_SPLIT_VIEW_SHOW_CONTENT_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showContent",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::NavigationSplitViewShowContent),
};

const NAVIGATION_SPLIT_VIEW_SIDEBAR_WIDTH_FRACTION_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "sidebarWidthFraction",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(
            GtkF64PropertySetter::NavigationSplitViewSidebarWidthFraction,
        ),
    };

const NAVIGATION_SPLIT_VIEW_MIN_SIDEBAR_WIDTH_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "minSidebarWidth",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(GtkF64PropertySetter::NavigationSplitViewMinSidebarWidth),
    };

const NAVIGATION_SPLIT_VIEW_MAX_SIDEBAR_WIDTH_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "maxSidebarWidth",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(GtkF64PropertySetter::NavigationSplitViewMaxSidebarWidth),
    };

const NAVIGATION_SPLIT_VIEW_SIDEBAR_POSITION_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "sidebarPosition",
        value_shape: GtkPropertyValueShape::Text,
        setter: GtkPropertySetter::Text(GtkTextPropertySetter::NavigationSplitViewSidebarPosition),
    };

const NAVIGATION_SPLIT_VIEW_SHOW_CONTENT_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onShowContent",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::NavigationSplitViewShowContentChanged,
};

const NAVIGATION_SPLIT_VIEW_SIDEBAR_CHILD_GROUP: GtkChildGroupDescriptor =
    GtkChildGroupDescriptor {
        name: "sidebar",
        container: GtkChildContainerKind::Single,
        mount: GtkChildMountRoute::NavigationSplitViewSidebar,
        min_children: 0,
        max_children: Some(1),
    };

const NAVIGATION_SPLIT_VIEW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor =
    GtkChildGroupDescriptor {
        name: "content",
        container: GtkChildContainerKind::Single,
        mount: GtkChildMountRoute::NavigationSplitViewContent,
        min_children: 0,
        max_children: Some(1),
    };

const NAVIGATION_SPLIT_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "NavigationSplitView",
    kind: GtkConcreteWidgetKind::NavigationSplitView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        NAVIGATION_SPLIT_VIEW_SHOW_CONTENT_PROPERTY,
        NAVIGATION_SPLIT_VIEW_SIDEBAR_WIDTH_FRACTION_PROPERTY,
        NAVIGATION_SPLIT_VIEW_MIN_SIDEBAR_WIDTH_PROPERTY,
        NAVIGATION_SPLIT_VIEW_MAX_SIDEBAR_WIDTH_PROPERTY,
        NAVIGATION_SPLIT_VIEW_SIDEBAR_POSITION_PROPERTY,
    ],
    events: &[NAVIGATION_SPLIT_VIEW_SHOW_CONTENT_EVENT],
    default_child_group_override: Some(&NAVIGATION_SPLIT_VIEW_CONTENT_CHILD_GROUP),
    child_groups: &[
        NAVIGATION_SPLIT_VIEW_SIDEBAR_CHILD_GROUP,
        NAVIGATION_SPLIT_VIEW_CONTENT_CHILD_GROUP,
    ],
};

// ── OverlaySplitView ──────────────────────────────────────────────────────────

const OVERLAY_SPLIT_VIEW_SHOW_SIDEBAR_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "showSidebar",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::OverlaySplitViewShowSidebar),
};

const OVERLAY_SPLIT_VIEW_SIDEBAR_POSITION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "sidebarPosition",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::OverlaySplitViewSidebarPosition),
};

const OVERLAY_SPLIT_VIEW_SIDEBAR_WIDTH_FRACTION_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "sidebarWidthFraction",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewSidebarWidthFraction),
    };

const OVERLAY_SPLIT_VIEW_MIN_SIDEBAR_WIDTH_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "minSidebarWidth",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewMinSidebarWidth),
    };

const OVERLAY_SPLIT_VIEW_MAX_SIDEBAR_WIDTH_PROPERTY: GtkPropertyDescriptor =
    GtkPropertyDescriptor {
        name: "maxSidebarWidth",
        value_shape: GtkPropertyValueShape::F64,
        setter: GtkPropertySetter::F64(GtkF64PropertySetter::OverlaySplitViewMaxSidebarWidth),
    };

const OVERLAY_SPLIT_VIEW_SHOW_SIDEBAR_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onShowSidebar",
    payload: GtkConcreteEventPayload::Bool,
    signal: GtkEventSignal::OverlaySplitViewShowSidebarChanged,
};

const OVERLAY_SPLIT_VIEW_SIDEBAR_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "sidebar",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::OverlaySplitViewSidebar,
    min_children: 0,
    max_children: Some(1),
};

const OVERLAY_SPLIT_VIEW_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::OverlaySplitViewContent,
    min_children: 0,
    max_children: Some(1),
};

const OVERLAY_SPLIT_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "OverlaySplitView",
    kind: GtkConcreteWidgetKind::OverlaySplitView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        OVERLAY_SPLIT_VIEW_SHOW_SIDEBAR_PROPERTY,
        OVERLAY_SPLIT_VIEW_SIDEBAR_POSITION_PROPERTY,
        OVERLAY_SPLIT_VIEW_SIDEBAR_WIDTH_FRACTION_PROPERTY,
        OVERLAY_SPLIT_VIEW_MIN_SIDEBAR_WIDTH_PROPERTY,
        OVERLAY_SPLIT_VIEW_MAX_SIDEBAR_WIDTH_PROPERTY,
    ],
    events: &[OVERLAY_SPLIT_VIEW_SHOW_SIDEBAR_EVENT],
    default_child_group_override: Some(&OVERLAY_SPLIT_VIEW_CONTENT_CHILD_GROUP),
    child_groups: &[
        OVERLAY_SPLIT_VIEW_SIDEBAR_CHILD_GROUP,
        OVERLAY_SPLIT_VIEW_CONTENT_CHILD_GROUP,
    ],
};

// ── TabView ───────────────────────────────────────────────────────────────────

const TAB_VIEW_SELECTED_PAGE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "selectedPage",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::TabViewSelectedPage),
};

const TAB_VIEW_PAGE_ADDED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPageAdded",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::TabViewPageAdded,
};

const TAB_VIEW_PAGE_CLOSED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPageClosed",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::TabViewPageClosed,
};

const TAB_VIEW_SELECTED_PAGE_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onSelectedPageChanged",
    payload: GtkConcreteEventPayload::Unit,
    signal: GtkEventSignal::TabViewSelectedPageChanged,
};

const TAB_VIEW_PAGES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "pages",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::TabViewPages,
    min_children: 0,
    max_children: None,
};

const TAB_VIEW_TAB_BAR_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "tabBar",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::TabViewTabBar,
    min_children: 0,
    max_children: Some(1),
};

const TAB_VIEW_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "TabView",
    kind: GtkConcreteWidgetKind::TabView,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        TAB_VIEW_SELECTED_PAGE_PROPERTY,
    ],
    events: &[
        TAB_VIEW_PAGE_ADDED_EVENT,
        TAB_VIEW_PAGE_CLOSED_EVENT,
        TAB_VIEW_SELECTED_PAGE_CHANGED_EVENT,
    ],
    default_child_group_override: Some(&TAB_VIEW_PAGES_CHILD_GROUP),
    child_groups: &[TAB_VIEW_PAGES_CHILD_GROUP, TAB_VIEW_TAB_BAR_CHILD_GROUP],
};

// ── TabPage ───────────────────────────────────────────────────────────────────

const TAB_PAGE_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::TabPageTitle),
};

const TAB_PAGE_NEEDS_ATTENTION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "needsAttention",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::TabPageNeedsAttention),
};

const TAB_PAGE_LOADING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "loading",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::TabPageLoading),
};

const TAB_PAGE_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::TabPageContent,
    min_children: 0,
    max_children: Some(1),
};

const TAB_PAGE_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "TabPage",
    kind: GtkConcreteWidgetKind::TabPage,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        TAB_PAGE_TITLE_PROPERTY,
        TAB_PAGE_NEEDS_ATTENTION_PROPERTY,
        TAB_PAGE_LOADING_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&TAB_PAGE_CONTENT_CHILD_GROUP),
    child_groups: &[TAB_PAGE_CONTENT_CHILD_GROUP],
};

// ── TabBar ────────────────────────────────────────────────────────────────────

const TAB_BAR_AUTOHIDE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "autohide",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::TabBarAutohide),
};

const TAB_BAR_EXPAND_TABS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "expandTabs",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::TabBarExpandTabs),
};

const TAB_BAR_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "TabBar",
    kind: GtkConcreteWidgetKind::TabBar,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        TAB_BAR_AUTOHIDE_PROPERTY,
        TAB_BAR_EXPAND_TABS_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Carousel ──────────────────────────────────────────────────────────────────

const CAROUSEL_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "spacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::CarouselSpacing),
};

const CAROUSEL_REVEAL_DURATION_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "revealDuration",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::CarouselRevealDuration),
};

const CAROUSEL_INTERACTIVE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "interactive",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::CarouselInteractive),
};

const CAROUSEL_PAGE_CHANGED_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onPageChanged",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::CarouselPageChanged,
};

const CAROUSEL_PAGES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "pages",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::CarouselPages,
    min_children: 0,
    max_children: None,
};

const CAROUSEL_DOTS_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "dots",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::CarouselDots,
    min_children: 0,
    max_children: Some(1),
};

const CAROUSEL_LINES_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "lines",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::CarouselLines,
    min_children: 0,
    max_children: Some(1),
};

const CAROUSEL_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Carousel",
    kind: GtkConcreteWidgetKind::Carousel,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        CAROUSEL_SPACING_PROPERTY,
        CAROUSEL_REVEAL_DURATION_PROPERTY,
        CAROUSEL_INTERACTIVE_PROPERTY,
    ],
    events: &[
        CAROUSEL_PAGE_CHANGED_EVENT,
        SWIPE_LEFT_EVENT,
        SWIPE_RIGHT_EVENT,
    ],
    default_child_group_override: Some(&CAROUSEL_PAGES_CHILD_GROUP),
    child_groups: &[
        CAROUSEL_PAGES_CHILD_GROUP,
        CAROUSEL_DOTS_CHILD_GROUP,
        CAROUSEL_LINES_CHILD_GROUP,
    ],
};

// ── CarouselIndicatorDots ─────────────────────────────────────────────────────

const CAROUSEL_INDICATOR_DOTS_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "CarouselIndicatorDots",
    kind: GtkConcreteWidgetKind::CarouselIndicatorDots,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

// ── CarouselIndicatorLines ────────────────────────────────────────────────────

const CAROUSEL_INDICATOR_LINES_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "CarouselIndicatorLines",
    kind: GtkConcreteWidgetKind::CarouselIndicatorLines,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
    ],
    events: &[],
    default_child_group_override: None,
    child_groups: &[],
};

// ── Grid ──────────────────────────────────────────────────────────────────────

const GRID_ROW_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "rowSpacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridRowSpacing),
};

const GRID_COLUMN_SPACING_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "columnSpacing",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridColumnSpacing),
};

const GRID_ROW_HOMOGENEOUS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "rowHomogeneous",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::GridRowHomogeneous),
};

const GRID_COLUMN_HOMOGENEOUS_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "columnHomogeneous",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::GridColumnHomogeneous),
};

const GRID_CHILDREN_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "children",
    container: GtkChildContainerKind::Sequence,
    mount: GtkChildMountRoute::GridChildren,
    min_children: 0,
    max_children: None,
};

const GRID_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "Grid",
    kind: GtkConcreteWidgetKind::Grid,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        GRID_ROW_SPACING_PROPERTY,
        GRID_COLUMN_SPACING_PROPERTY,
        GRID_ROW_HOMOGENEOUS_PROPERTY,
        GRID_COLUMN_HOMOGENEOUS_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&GRID_CHILDREN_CHILD_GROUP),
    child_groups: &[GRID_CHILDREN_CHILD_GROUP],
};

// ── GridChild ─────────────────────────────────────────────────────────────────

const GRID_CHILD_COLUMN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "column",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridChildColumn),
};

const GRID_CHILD_ROW_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "row",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridChildRow),
};

const GRID_CHILD_COLUMN_SPAN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "columnSpan",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridChildColumnSpan),
};

const GRID_CHILD_ROW_SPAN_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "rowSpan",
    value_shape: GtkPropertyValueShape::I64,
    setter: GtkPropertySetter::I64(GtkI64PropertySetter::GridChildRowSpan),
};

const GRID_CHILD_CONTENT_CHILD_GROUP: GtkChildGroupDescriptor = GtkChildGroupDescriptor {
    name: "content",
    container: GtkChildContainerKind::Single,
    mount: GtkChildMountRoute::GridChildContent,
    min_children: 0,
    max_children: Some(1),
};

const GRID_CHILD_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "GridChild",
    kind: GtkConcreteWidgetKind::GridChild,
    root_kind: GtkWidgetRootKind::Embedded,
    properties: &[
        VISIBLE_PROPERTY,
        SENSITIVE_PROPERTY,
        HEXPAND_PROPERTY,
        VEXPAND_PROPERTY,
        OPACITY_PROPERTY,
        ANIMATE_OPACITY_PROPERTY,
        WIDTH_REQUEST_PROPERTY,
        HEIGHT_REQUEST_PROPERTY,
        HALIGN_PROPERTY,
        VALIGN_PROPERTY,
        MARGIN_START_PROPERTY,
        MARGIN_END_PROPERTY,
        MARGIN_TOP_PROPERTY,
        MARGIN_BOTTOM_PROPERTY,
        TOOLTIP_PROPERTY,
        CSS_CLASSES_PROPERTY,
        GRID_CHILD_COLUMN_PROPERTY,
        GRID_CHILD_ROW_PROPERTY,
        GRID_CHILD_COLUMN_SPAN_PROPERTY,
        GRID_CHILD_ROW_SPAN_PROPERTY,
    ],
    events: &[],
    default_child_group_override: Some(&GRID_CHILD_CONTENT_CHILD_GROUP),
    child_groups: &[GRID_CHILD_CONTENT_CHILD_GROUP],
};

// ── FileDialog ────────────────────────────────────────────────────────────────

const FILE_DIALOG_VISIBLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "visible",
    value_shape: GtkPropertyValueShape::Bool,
    setter: GtkPropertySetter::Bool(GtkBoolPropertySetter::FileDialogVisible),
};

const FILE_DIALOG_TITLE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "title",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogTitle),
};

const FILE_DIALOG_MODE_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "mode",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogMode),
};

const FILE_DIALOG_ACCEPT_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "acceptLabel",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogAcceptLabel),
};

const FILE_DIALOG_CANCEL_LABEL_PROPERTY: GtkPropertyDescriptor = GtkPropertyDescriptor {
    name: "cancelLabel",
    value_shape: GtkPropertyValueShape::Text,
    setter: GtkPropertySetter::Text(GtkTextPropertySetter::FileDialogCancelLabel),
};

const FILE_DIALOG_RESPONSE_EVENT: GtkEventDescriptor = GtkEventDescriptor {
    name: "onResponse",
    payload: GtkConcreteEventPayload::I64,
    signal: GtkEventSignal::FileDialogResponse,
};

const FILE_DIALOG_SCHEMA: GtkWidgetSchema = GtkWidgetSchema {
    markup_name: "FileDialog",
    kind: GtkConcreteWidgetKind::FileDialog,
    root_kind: GtkWidgetRootKind::Window,
    properties: &[
        FILE_DIALOG_VISIBLE_PROPERTY,
        FILE_DIALOG_TITLE_PROPERTY,
        FILE_DIALOG_MODE_PROPERTY,
        FILE_DIALOG_ACCEPT_LABEL_PROPERTY,
        FILE_DIALOG_CANCEL_LABEL_PROPERTY,
    ],
    events: &[FILE_DIALOG_RESPONSE_EVENT],
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
    SPIN_BUTTON_SCHEMA,
    SCALE_SCHEMA,
    IMAGE_SCHEMA,
    SPINNER_SCHEMA,
    PROGRESS_BAR_SCHEMA,
    REVEALER_SCHEMA,
    SEPARATOR_SCHEMA,
    STATUS_PAGE_SCHEMA,
    CLAMP_SCHEMA,
    BANNER_SCHEMA,
    TOOLBAR_VIEW_SCHEMA,
    ACTION_ROW_SCHEMA,
    EXPANDER_ROW_SCHEMA,
    SWITCH_ROW_SCHEMA,
    SPIN_ROW_SCHEMA,
    ENTRY_ROW_SCHEMA,
    LIST_BOX_SCHEMA,
    LIST_BOX_ROW_SCHEMA,
    LIST_VIEW_SCHEMA,
    GRID_VIEW_SCHEMA,
    DROP_DOWN_SCHEMA,
    SEARCH_ENTRY_SCHEMA,
    EXPANDER_SCHEMA,
    NAVIGATION_VIEW_SCHEMA,
    NAVIGATION_PAGE_SCHEMA,
    TOAST_OVERLAY_SCHEMA,
    PREFERENCES_GROUP_SCHEMA,
    PREFERENCES_PAGE_SCHEMA,
    PREFERENCES_WINDOW_SCHEMA,
    COMBO_ROW_SCHEMA,
    PASSWORD_ENTRY_ROW_SCHEMA,
    OVERLAY_SCHEMA,
    MULTILINE_ENTRY_SCHEMA,
    PICTURE_SCHEMA,
    WEB_VIEW_SCHEMA,
    VIEW_STACK_SCHEMA,
    VIEW_STACK_PAGE_SCHEMA,
    ALERT_DIALOG_SCHEMA,
    CALENDAR_SCHEMA,
    FLOW_BOX_SCHEMA,
    FLOW_BOX_CHILD_SCHEMA,
    MENU_BUTTON_SCHEMA,
    POPOVER_SCHEMA,
    CENTER_BOX_SCHEMA,
    ABOUT_DIALOG_SCHEMA,
    SPLIT_BUTTON_SCHEMA,
    NAVIGATION_SPLIT_VIEW_SCHEMA,
    OVERLAY_SPLIT_VIEW_SCHEMA,
    TAB_VIEW_SCHEMA,
    TAB_PAGE_SCHEMA,
    TAB_BAR_SCHEMA,
    CAROUSEL_SCHEMA,
    CAROUSEL_INDICATOR_DOTS_SCHEMA,
    CAROUSEL_INDICATOR_LINES_SCHEMA,
    GRID_SCHEMA,
    GRID_CHILD_SCHEMA,
    FILE_DIALOG_SCHEMA,
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
                "SpinButton",
                "Scale",
                "Image",
                "Spinner",
                "ProgressBar",
                "Revealer",
                "Separator",
                "StatusPage",
                "Clamp",
                "Banner",
                "ToolbarView",
                "ActionRow",
                "ExpanderRow",
                "SwitchRow",
                "SpinRow",
                "EntryRow",
                "ListBox",
                "ListBoxRow",
                "ListView",
                "GridView",
                "DropDown",
                "SearchEntry",
                "Expander",
                "NavigationView",
                "NavigationPage",
                "ToastOverlay",
                "PreferencesGroup",
                "PreferencesPage",
                "PreferencesWindow",
                "ComboRow",
                "PasswordEntryRow",
                "Overlay",
                "MultilineEntry",
                "Picture",
                "WebView",
                "ViewStack",
                "ViewStackPage",
                "AlertDialog",
                "Calendar",
                "FlowBox",
                "FlowBoxChild",
                "MenuButton",
                "Popover",
                "CenterBox",
                "AboutDialog",
                "SplitButton",
                "NavigationSplitView",
                "OverlaySplitView",
                "TabView",
                "TabPage",
                "TabBar",
                "Carousel",
                "CarouselIndicatorDots",
                "CarouselIndicatorLines",
                "Grid",
                "GridChild",
                "FileDialog",
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
        let list_view = path(&["ListView"]);
        let grid_view = path(&["GridView"]);
        let property = lookup_widget_property(&button, "label")
            .expect("Button.label should be part of the catalog");
        assert_eq!(property.value_shape, GtkPropertyValueShape::Text);
        assert!(lookup_widget_property(&button, "text").is_none());
        assert!(lookup_widget_property(&button, "compact").is_some());
        assert!(lookup_widget_property(&button, "hasFrame").is_some());
        assert!(lookup_widget_property(&button, "focusable").is_some());
        assert!(lookup_widget_property(&button, "widthRequest").is_some());
        assert!(lookup_widget_property(&button, "heightRequest").is_some());
        assert!(lookup_widget_property(&button, "animateOpacity").is_some());
        assert!(lookup_widget_property(&button, "opacity").is_some());
        assert!(lookup_widget_property(&label, "label").is_some());
        assert!(lookup_widget_property(&label, "monospace").is_some());
        assert!(lookup_widget_property(&path(&["WebView"]), "html").is_some());
        assert!(lookup_widget_property(&list_view, "showSeparators").is_some());
        assert!(lookup_widget_property(&list_view, "singleClickActivate").is_some());
        assert!(lookup_widget_property(&grid_view, "minColumns").is_some());
        assert!(lookup_widget_property(&grid_view, "maxColumns").is_some());
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
        let list_view = path(&["ListView"]);
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
        let event = lookup_widget_event(&list_view, "onActivate")
            .expect("ListView.onActivate should be in catalog");
        assert_eq!(event.payload, GtkConcreteEventPayload::I64);
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

        let list_view =
            lookup_widget_schema_by_name("ListView").expect("ListView schema should exist");
        assert!(matches!(
            list_view.default_child_group(),
            GtkDefaultChildGroup::One(group)
                if group.name == "children"
                    && group.container == GtkChildContainerKind::Sequence
                    && group.accepts_child_count(0)
                    && group.accepts_child_count(2)
        ));

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
