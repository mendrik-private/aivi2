use std::fmt;

use aivi_hir::BuiltinType;

use crate::LayoutId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbiPassMode {
    ByValue,
    ByReference,
}

impl fmt::Display for AbiPassMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByValue => f.write_str("by-value"),
            Self::ByReference => f.write_str("by-reference"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    Int,
    Float,
    Decimal,
    BigInt,
    Bool,
    Text,
    Unit,
    Bytes,
    List,
    Map,
    Set,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

impl PrimitiveType {
    pub const fn from_builtin(builtin: BuiltinType) -> Self {
        match builtin {
            BuiltinType::Int => Self::Int,
            BuiltinType::Float => Self::Float,
            BuiltinType::Decimal => Self::Decimal,
            BuiltinType::BigInt => Self::BigInt,
            BuiltinType::Bool => Self::Bool,
            BuiltinType::Text => Self::Text,
            BuiltinType::Unit => Self::Unit,
            BuiltinType::Bytes => Self::Bytes,
            BuiltinType::List => Self::List,
            BuiltinType::Map => Self::Map,
            BuiltinType::Set => Self::Set,
            BuiltinType::Option => Self::Option,
            BuiltinType::Result => Self::Result,
            BuiltinType::Validation => Self::Validation,
            BuiltinType::Signal => Self::Signal,
            BuiltinType::Task => Self::Task,
        }
    }

    pub const fn default_abi(self) -> AbiPassMode {
        match self {
            Self::Int | Self::Float | Self::Bool | Self::Unit => AbiPassMode::ByValue,
            Self::Decimal | Self::BigInt => AbiPassMode::ByReference,
            Self::Text
            | Self::Bytes
            | Self::List
            | Self::Map
            | Self::Set
            | Self::Option
            | Self::Result
            | Self::Validation
            | Self::Signal
            | Self::Task => AbiPassMode::ByReference,
        }
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int => f.write_str("Int"),
            Self::Float => f.write_str("Float"),
            Self::Decimal => f.write_str("Decimal"),
            Self::BigInt => f.write_str("BigInt"),
            Self::Bool => f.write_str("Bool"),
            Self::Text => f.write_str("Text"),
            Self::Unit => f.write_str("Unit"),
            Self::Bytes => f.write_str("Bytes"),
            Self::List => f.write_str("List"),
            Self::Map => f.write_str("Map"),
            Self::Set => f.write_str("Set"),
            Self::Option => f.write_str("Option"),
            Self::Result => f.write_str("Result"),
            Self::Validation => f.write_str("Validation"),
            Self::Signal => f.write_str("Signal"),
            Self::Task => f.write_str("Task"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordFieldLayout {
    pub name: Box<str>,
    pub layout: LayoutId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariantLayout {
    pub name: Box<str>,
    pub payload: Option<LayoutId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayoutKind {
    Primitive(PrimitiveType),
    Tuple(Vec<LayoutId>),
    Record(Vec<RecordFieldLayout>),
    Sum(Vec<VariantLayout>),
    Arrow {
        parameter: LayoutId,
        result: LayoutId,
    },
    List {
        element: LayoutId,
    },
    Map {
        key: LayoutId,
        value: LayoutId,
    },
    Set {
        element: LayoutId,
    },
    Option {
        element: LayoutId,
    },
    Result {
        error: LayoutId,
        value: LayoutId,
    },
    Validation {
        error: LayoutId,
        value: LayoutId,
    },
    Signal {
        element: LayoutId,
    },
    Task {
        error: LayoutId,
        value: LayoutId,
    },
    AnonymousDomain {
        carrier: LayoutId,
        surface_member: Box<str>,
    },
    Domain {
        name: Box<str>,
        arguments: Vec<LayoutId>,
    },
    Opaque {
        name: Box<str>,
        arguments: Vec<LayoutId>,
    },
}

impl LayoutKind {
    pub const fn default_abi(&self) -> AbiPassMode {
        match self {
            Self::Primitive(primitive) => primitive.default_abi(),
            Self::Tuple(_)
            | Self::Record(_)
            | Self::Sum(_)
            | Self::Arrow { .. }
            | Self::List { .. }
            | Self::Map { .. }
            | Self::Set { .. }
            | Self::Option { .. }
            | Self::Result { .. }
            | Self::Validation { .. }
            | Self::Signal { .. }
            | Self::Task { .. }
            | Self::AnonymousDomain { .. }
            | Self::Domain { .. }
            | Self::Opaque { .. } => AbiPassMode::ByReference,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    pub abi: AbiPassMode,
    pub kind: LayoutKind,
}

impl Layout {
    pub fn new(kind: LayoutKind) -> Self {
        let abi = kind.default_abi();
        Self { abi, kind }
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            LayoutKind::Primitive(primitive) => write!(f, "{primitive} [{}]", self.abi),
            LayoutKind::Tuple(elements) => {
                write!(f, "tuple(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "layout{element}")?;
                }
                write!(f, ") [{}]", self.abi)
            }
            LayoutKind::Record(fields) => {
                f.write_str("record {")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: layout{}", field.name, field.layout)?;
                }
                write!(f, "}} [{}]", self.abi)
            }
            LayoutKind::Sum(variants) => {
                f.write_str("sum(")?;
                for (index, variant) in variants.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", variant.name)?;
                    if let Some(payload) = variant.payload {
                        write!(f, " layout{payload}")?;
                    }
                }
                write!(f, ") [{}]", self.abi)
            }
            LayoutKind::Arrow { parameter, result } => {
                write!(
                    f,
                    "arrow(layout{parameter} -> layout{result}) [{}]",
                    self.abi
                )
            }
            LayoutKind::List { element } => write!(f, "List layout{element} [{}]", self.abi),
            LayoutKind::Map { key, value } => {
                write!(f, "Map layout{key} layout{value} [{}]", self.abi)
            }
            LayoutKind::Set { element } => write!(f, "Set layout{element} [{}]", self.abi),
            LayoutKind::Option { element } => {
                write!(f, "Option layout{element} [{}]", self.abi)
            }
            LayoutKind::Result { error, value } => {
                write!(f, "Result layout{error} layout{value} [{}]", self.abi)
            }
            LayoutKind::Validation { error, value } => {
                write!(f, "Validation layout{error} layout{value} [{}]", self.abi)
            }
            LayoutKind::Signal { element } => {
                write!(f, "Signal layout{element} [{}]", self.abi)
            }
            LayoutKind::Task { error, value } => {
                write!(f, "Task layout{error} layout{value} [{}]", self.abi)
            }
            LayoutKind::AnonymousDomain {
                carrier,
                surface_member,
            } => write!(
                f,
                "anonymous-domain via {} carrier=layout{} [{}]",
                surface_member, carrier, self.abi
            ),
            LayoutKind::Domain { name, arguments } | LayoutKind::Opaque { name, arguments } => {
                write!(f, "{name}")?;
                for argument in arguments {
                    write!(f, " layout{argument}")?;
                }
                write!(f, " [{}]", self.abi)
            }
        }
    }
}
