#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ApplicativeClusterKind {
    List,
    Option,
    Result { error: SourceOptionActualType },
    Validation { error: SourceOptionActualType },
    Signal,
    Task { error: SourceOptionActualType },
}

impl ApplicativeClusterKind {
    pub(crate) fn from_member_actual(
        actual: &SourceOptionActualType,
    ) -> Option<(Self, SourceOptionActualType)> {
        match actual {
            SourceOptionActualType::List(element) => Some((Self::List, element.as_ref().clone())),
            SourceOptionActualType::Option(element) => {
                Some((Self::Option, element.as_ref().clone()))
            }
            SourceOptionActualType::Result { error, value } => Some((
                Self::Result {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Validation { error, value } => Some((
                Self::Validation {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Signal(element) => {
                Some((Self::Signal, element.as_ref().clone()))
            }
            SourceOptionActualType::Task { error, value } => Some((
                Self::Task {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Hole
            | SourceOptionActualType::Primitive(_)
            | SourceOptionActualType::Tuple(_)
            | SourceOptionActualType::Record(_)
            | SourceOptionActualType::Arrow { .. }
            | SourceOptionActualType::Map { .. }
            | SourceOptionActualType::Set(_)
            | SourceOptionActualType::Domain { .. }
            | SourceOptionActualType::OpaqueItem { .. }
            | SourceOptionActualType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::List, Self::List) => Some(Self::List),
            (Self::Option, Self::Option) => Some(Self::Option),
            (Self::Signal, Self::Signal) => Some(Self::Signal),
            (Self::Result { error: left }, Self::Result { error: right }) => Some(Self::Result {
                error: left.unify(right)?,
            }),
            (Self::Validation { error: left }, Self::Validation { error: right }) => {
                Some(Self::Validation {
                    error: left.unify(right)?,
                })
            }
            (Self::Task { error: left }, Self::Task { error: right }) => Some(Self::Task {
                error: left.unify(right)?,
            }),
            _ => None,
        }
    }

    pub(crate) fn wrap_actual(&self, payload: SourceOptionActualType) -> SourceOptionActualType {
        match self {
            Self::List => SourceOptionActualType::List(Box::new(payload)),
            Self::Option => SourceOptionActualType::Option(Box::new(payload)),
            Self::Result { error } => SourceOptionActualType::Result {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Validation { error } => SourceOptionActualType::Validation {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Signal => SourceOptionActualType::Signal(Box::new(payload)),
            Self::Task { error } => SourceOptionActualType::Task {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
        }
    }

    pub(crate) fn surface(&self) -> String {
        match self {
            Self::List => "List _".to_owned(),
            Self::Option => "Option _".to_owned(),
            Self::Result { error } => format!("Result {error} _"),
            Self::Validation { error } => format!("Validation {error} _"),
            Self::Signal => "Signal _".to_owned(),
            Self::Task { error } => format!("Task {error} _"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeBinding {
    Type(GateType),
    Constructor(TypeConstructorBinding),
}

impl TypeBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Type(left), Self::Type(right)) => left.same_shape(right),
            (Self::Constructor(left), Self::Constructor(right)) => left.matches(right),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeConstructorBinding {
    head: TypeConstructorHead,
    arguments: Vec<GateType>,
}

impl TypeConstructorBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.head == other.head
            && self.arguments.len() == other.arguments.len()
            && self
                .arguments
                .iter()
                .zip(other.arguments.iter())
                .all(|(left, right)| left.same_shape(right))
    }

    pub fn head(&self) -> TypeConstructorHead {
        self.head
    }

    pub fn arguments(&self) -> &[GateType] {
        &self.arguments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeConstructorHead {
    Builtin(BuiltinType),
    Item(ItemId),
    Import(ImportId),
}

pub(crate) type PolyTypeBindings = HashMap<TypeParameterId, TypeBinding>;
