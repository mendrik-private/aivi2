#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionExpectedType {
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionExpectedRecordField>),
    List(Box<Self>),
    Map { key: Box<Self>, value: Box<Self> },
    Set(Box<Self>),
    Signal(Box<Self>),
    Option(Box<Self>),
    Result { error: Box<Self>, value: Box<Self> },
    Validation { error: Box<Self>, value: Box<Self> },
    Named(SourceOptionNamedType),
    ContractParameter(SourceTypeParameter),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionExpectedRecordField {
    pub(crate) name: String,
    pub(crate) ty: SourceOptionExpectedType,
}

/// Local proof type that keeps builtin container holes explicit until later
/// ordinary-expression or source-option evidence refines them into closed `GateType`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionActualType {
    Hole,
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionActualRecordField>),
    Arrow {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    List(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
    },
    Set(Box<Self>),
    Option(Box<Self>),
    Result {
        error: Box<Self>,
        value: Box<Self>,
    },
    Validation {
        error: Box<Self>,
        value: Box<Self>,
    },
    Signal(Box<Self>),
    Task {
        error: Box<Self>,
        value: Box<Self>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
    OpaqueImport {
        import: ImportId,
        name: String,
        arguments: Vec<Self>,
        definition: Option<Box<ImportTypeDefinition>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionActualRecordField {
    pub(crate) name: String,
    pub(crate) ty: SourceOptionActualType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionTypeSurface {
    Contract,
    Expression,
}

impl SourceOptionExpectedType {
    pub(crate) fn from_resolved(module: &Module, ty: &ResolvedSourceContractType) -> Option<Self> {
        match ty {
            ResolvedSourceContractType::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            ) => Some(Self::Primitive(*builtin)),
            ResolvedSourceContractType::Builtin(_) => None,
            ResolvedSourceContractType::ContractParameter(parameter) => {
                Some(Self::ContractParameter(*parameter))
            }
            ResolvedSourceContractType::Item(item) => Some(Self::Named(
                SourceOptionNamedType::from_item(module, *item, Vec::new())?,
            )),
            ResolvedSourceContractType::Apply { callee, arguments } => match callee {
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::List) => Some(Self::List(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Map) => Some(Self::Map {
                    key: Box::new(Self::from_resolved(module, arguments.first()?)?),
                    value: Box::new(Self::from_resolved(module, arguments.get(1)?)?),
                }),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Set) => Some(Self::Set(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal) => Some(Self::Signal(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(_) => None,
                ResolvedSourceTypeConstructor::Item(item) => {
                    let arguments = arguments
                        .iter()
                        .map(|argument| Self::from_resolved(module, argument))
                        .collect::<Option<Vec<_>>>()?;
                    Some(Self::Named(SourceOptionNamedType::from_item(
                        module, *item, arguments,
                    )?))
                }
            },
        }
    }

    pub(crate) fn from_hir_type(
        module: &Module,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, SourceOptionExpectedType>,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match &module.types()[ty].kind {
            TypeKind::RecordTransform { .. } => None,
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Builtin(
                    builtin @ (BuiltinType::Int
                    | BuiltinType::Float
                    | BuiltinType::Decimal
                    | BuiltinType::BigInt
                    | BuiltinType::Bool
                    | BuiltinType::Text
                    | BuiltinType::Unit
                    | BuiltinType::Bytes),
                )) => Some(Self::Primitive(*builtin)),
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    substitutions.get(parameter).cloned()
                }
                ResolutionState::Resolved(TypeResolution::Item(item)) => Some(Self::Named(
                    SourceOptionNamedType::from_item(module, *item, Vec::new())?,
                )),
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                        Some(Self::List(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Map {
                            key: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Set(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                        Some(Self::Signal(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Option(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Result {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Validation {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Item(item)) => {
                        let arguments = arguments
                            .iter()
                            .map(|argument| {
                                Self::from_hir_type(module, *argument, substitutions, surface)
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some(Self::Named(SourceOptionNamedType::from_item(
                            module, *item, arguments,
                        )?))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(_))
                    | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
                    | ResolutionState::Resolved(TypeResolution::Import(_))
                    | ResolutionState::Unresolved => None,
                }
            }
            TypeKind::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .copied()
                    .map(|element| Self::from_hir_type(module, element, substitutions, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.label.text().to_owned(),
                            ty: Self::from_hir_type(module, field.ty, substitutions, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn from_gate_type(
        module: &Module,
        ty: &GateType,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match ty {
            GateType::Primitive(builtin) => Some(Self::Primitive(*builtin)),
            GateType::TypeParameter { .. } => None,
            GateType::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .map(|element| Self::from_gate_type(module, element, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.name.clone(),
                            ty: Self::from_gate_type(module, &field.ty, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::List(element) => Some(Self::List(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Map { key, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Map {
                    key: Box::new(Self::from_gate_type(module, key, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Set(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Set(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Signal(element) => Some(Self::Signal(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Option(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Option(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Result { error, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Result {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Validation { error, value }
                if surface == SourceOptionTypeSurface::Expression =>
            {
                Some(Self::Validation {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Domain {
                item, arguments, ..
            }
            | GateType::OpaqueItem {
                item, arguments, ..
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| Self::from_gate_type(module, argument, surface))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Named(SourceOptionNamedType::from_item(
                    module, *item, arguments,
                )?))
            }
            GateType::Arrow { .. }
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Option(_)
            | GateType::Result { .. }
            | GateType::Validation { .. }
            | GateType::Task { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn is_signal_contract(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn matches_named_item(&self, item: ItemId) -> bool {
        matches!(self, Self::Named(named) if named.item == item)
    }

    pub(crate) fn as_named(&self) -> Option<&SourceOptionNamedType> {
        let Self::Named(named) = self else {
            return None;
        };
        Some(named)
    }
}

impl SourceOptionActualType {
    pub(crate) fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn from_gate_type(ty: &GateType) -> Self {
        match ty {
            GateType::Primitive(builtin) => Self::Primitive(*builtin),
            GateType::TypeParameter { .. } => Self::Hole,
            GateType::Tuple(elements) => {
                Self::Tuple(elements.iter().map(Self::from_gate_type).collect())
            }
            GateType::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|field| SourceOptionActualRecordField {
                        name: field.name.clone(),
                        ty: Self::from_gate_type(&field.ty),
                    })
                    .collect(),
            ),
            GateType::Arrow { parameter, result } => Self::Arrow {
                parameter: Box::new(Self::from_gate_type(parameter)),
                result: Box::new(Self::from_gate_type(result)),
            },
            GateType::List(element) => Self::List(Box::new(Self::from_gate_type(element))),
            GateType::Map { key, value } => Self::Map {
                key: Box::new(Self::from_gate_type(key)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Set(element) => Self::Set(Box::new(Self::from_gate_type(element))),
            GateType::Option(element) => Self::Option(Box::new(Self::from_gate_type(element))),
            GateType::Result { error, value } => Self::Result {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Validation { error, value } => Self::Validation {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Signal(element) => Self::Signal(Box::new(Self::from_gate_type(element))),
            GateType::Task { error, value } => Self::Task {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Domain {
                item,
                name,
                arguments,
            } => Self::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
            GateType::OpaqueItem {
                item,
                name,
                arguments,
            } => Self::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
            GateType::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Self::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
                definition: definition.clone(),
            },
        }
    }

    pub(crate) fn to_gate_type(&self) -> Option<GateType> {
        match self {
            Self::Hole => None,
            Self::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
            Self::Tuple(elements) => Some(GateType::Tuple(
                elements
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Record(fields) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(GateRecordField {
                            name: field.name.clone(),
                            ty: field.ty.to_gate_type()?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(parameter.to_gate_type()?),
                result: Box::new(result.to_gate_type()?),
            }),
            Self::List(element) => Some(GateType::List(Box::new(element.to_gate_type()?))),
            Self::Map { key, value } => Some(GateType::Map {
                key: Box::new(key.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Set(element) => Some(GateType::Set(Box::new(element.to_gate_type()?))),
            Self::Option(element) => Some(GateType::Option(Box::new(element.to_gate_type()?))),
            Self::Result { error, value } => Some(GateType::Result {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Validation { error, value } => Some(GateType::Validation {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Signal(element) => Some(GateType::Signal(Box::new(element.to_gate_type()?))),
            Self::Task { error, value } => Some(GateType::Task {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Domain {
                item,
                name,
                arguments,
            } => Some(GateType::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
            Self::OpaqueItem {
                item,
                name,
                arguments,
            } => Some(GateType::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
            Self::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Some(GateType::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
                definition: definition.clone(),
            }),
        }
    }

    pub(crate) fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Hole, actual) | (actual, Self::Hole) => Some(actual.clone()),
            (Self::Primitive(left), Self::Primitive(right)) if left == right => {
                Some(Self::Primitive(*left))
            }
            (Self::Tuple(left), Self::Tuple(right)) if left.len() == right.len() => {
                Some(Self::Tuple(
                    left.iter()
                        .zip(right)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                ))
            }
            (Self::Record(left), Self::Record(right)) if left.len() == right.len() => {
                let right_fields = right
                    .iter()
                    .map(|field| (field.name.as_str(), field))
                    .collect::<HashMap<_, _>>();
                let mut fields = Vec::with_capacity(left.len());
                for left in left {
                    let right = right_fields.get(left.name.as_str())?;
                    fields.push(SourceOptionActualRecordField {
                        name: left.name.clone(),
                        ty: left.ty.unify(&right.ty)?,
                    });
                }
                Some(Self::Record(fields))
            }
            (
                Self::Arrow {
                    parameter: left_parameter,
                    result: left_result,
                },
                Self::Arrow {
                    parameter: right_parameter,
                    result: right_result,
                },
            ) => Some(Self::Arrow {
                parameter: Box::new(left_parameter.unify(right_parameter)?),
                result: Box::new(left_result.unify(right_result)?),
            }),
            (Self::List(left), Self::List(right)) => Some(Self::List(Box::new(left.unify(right)?))),
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => Some(Self::Map {
                key: Box::new(left_key.unify(right_key)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Set(left), Self::Set(right)) => Some(Self::Set(Box::new(left.unify(right)?))),
            (Self::Option(left), Self::Option(right)) => {
                Some(Self::Option(Box::new(left.unify(right)?)))
            }
            (
                Self::Result {
                    error: left_error,
                    value: left_value,
                },
                Self::Result {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Result {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Validation {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Signal(left), Self::Signal(right)) => {
                Some(Self::Signal(Box::new(left.unify(right)?)))
            }
            (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Task {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Domain {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::Domain {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::Domain {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            (
                Self::OpaqueItem {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::OpaqueItem {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            (
                Self::OpaqueImport {
                    import: left_import,
                    name,
                    arguments: left_arguments,
                    definition,
                },
                Self::OpaqueImport {
                    import: right_import,
                    arguments: right_arguments,
                    ..
                },
            ) if left_import == right_import && left_arguments.len() == right_arguments.len() => {
                Some(Self::OpaqueImport {
                    import: *left_import,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                    definition: definition.clone(),
                })
            }
            _ => None,
        }
    }
}

impl fmt::Display for SourceOptionActualType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hole => write!(f, "_"),
            Self::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            Self::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            Self::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            Self::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            Self::List(element) => write!(f, "List {element}"),
            Self::Map { key, value } => write!(f, "Map {key} {value}"),
            Self::Set(element) => write!(f, "Set {element}"),
            Self::Option(element) => write!(f, "Option {element}"),
            Self::Result { error, value } => write!(f, "Result {error} {value}"),
            Self::Validation { error, value } => write!(f, "Validation {error} {value}"),
            Self::Signal(element) => write!(f, "Signal {element}"),
            Self::Task { error, value } => write!(f, "Task {error} {value}"),
            Self::Domain {
                name, arguments, ..
            }
            | Self::OpaqueItem {
                name, arguments, ..
            }
            | Self::OpaqueImport {
                name, arguments, ..
            } => {
                if arguments.is_empty() {
                    write!(f, "{name}")
                } else {
                    write!(f, "{name}")?;
                    for argument in arguments {
                        write!(f, " {argument}")?;
                    }
                    Ok(())
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionNamedType {
    pub(crate) item: ItemId,
    pub(crate) name: String,
    pub(crate) kind: SourceOptionNamedKind,
    pub(crate) arguments: Vec<SourceOptionExpectedType>,
}

impl SourceOptionNamedType {
    pub(crate) fn from_item(
        module: &Module,
        item: ItemId,
        arguments: Vec<SourceOptionExpectedType>,
    ) -> Option<Self> {
        let item_ref = &module.items()[item];
        let kind = match item_ref {
            Item::Domain(_) => SourceOptionNamedKind::Domain,
            Item::Type(_) => SourceOptionNamedKind::Type,
            Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::Class(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => return None,
        };
        Some(Self {
            item,
            name: item_type_name(item_ref),
            kind,
            arguments,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionNamedKind {
    Domain,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionConstructorActual {
    pub(crate) parent_item: ItemId,
    pub(crate) parent_name: String,
    pub(crate) constructor_name: String,
    pub(crate) field_types: Vec<TypeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceOptionTypeBindings {
    pub(crate) parameters: HashMap<SourceTypeParameter, SourceOptionActualType>,
}

impl SourceOptionTypeBindings {
    pub(crate) fn parameter(
        &self,
        parameter: SourceTypeParameter,
    ) -> Option<&SourceOptionActualType> {
        self.parameters.get(&parameter)
    }

    pub(crate) fn parameter_gate_type(&self, parameter: SourceTypeParameter) -> Option<GateType> {
        self.parameter(parameter)?.to_gate_type()
    }

    pub(crate) fn bind_or_match_actual(
        &mut self,
        parameter: SourceTypeParameter,
        actual: &SourceOptionActualType,
    ) -> bool {
        match self.parameters.entry(parameter) {
            Entry::Occupied(mut entry) => {
                let Some(unified) = entry.get().unify(actual) else {
                    return false;
                };
                entry.insert(unified);
                true
            }
            Entry::Vacant(entry) => {
                entry.insert(actual.clone());
                true
            }
        }
    }
}

pub(crate) fn source_option_contract_parameters(
    expected: &SourceOptionExpectedType,
) -> Vec<SourceTypeParameter> {
    pub(crate) fn collect(
        expected: &SourceOptionExpectedType,
        parameters: &mut Vec<SourceTypeParameter>,
    ) {
        match expected {
            SourceOptionExpectedType::Primitive(_) => {}
            SourceOptionExpectedType::Tuple(elements) => {
                for element in elements {
                    collect(element, parameters);
                }
            }
            SourceOptionExpectedType::Record(fields) => {
                for field in fields {
                    collect(&field.ty, parameters);
                }
            }
            SourceOptionExpectedType::List(element)
            | SourceOptionExpectedType::Set(element)
            | SourceOptionExpectedType::Signal(element)
            | SourceOptionExpectedType::Option(element) => collect(element, parameters),
            SourceOptionExpectedType::Map { key, value }
            | SourceOptionExpectedType::Result { error: key, value }
            | SourceOptionExpectedType::Validation { error: key, value } => {
                collect(key, parameters);
                collect(value, parameters);
            }
            SourceOptionExpectedType::Named(named) => {
                for argument in &named.arguments {
                    collect(argument, parameters);
                }
            }
            SourceOptionExpectedType::ContractParameter(parameter) => {
                if !parameters.contains(parameter) {
                    parameters.push(*parameter);
                }
            }
        }
    }

    let mut parameters = Vec::new();
    collect(expected, &mut parameters);
    parameters
}

pub(crate) fn source_option_unresolved_contract_parameters(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Vec<SourceTypeParameter> {
    source_option_contract_parameters(expected)
        .into_iter()
        .filter(|parameter| bindings.parameter_gate_type(*parameter).is_none())
        .collect()
}

pub(crate) fn source_option_contract_parameter_phrase(
    parameters: &[SourceTypeParameter],
) -> String {
    let quoted = parameters
        .iter()
        .map(|parameter| format!("`{parameter}`"))
        .collect::<Vec<_>>();
    match quoted.as_slice() {
        [] => "contract parameters".to_owned(),
        [single] => format!("contract parameter {single}"),
        [left, right] => format!("contract parameters {left} and {right}"),
        _ => format!(
            "contract parameters {}, and {}",
            quoted[..quoted.len() - 1].join(", "),
            quoted
                .last()
                .expect("non-empty parameter list should keep a tail"),
        ),
    }
}

pub(crate) fn source_option_expected_to_gate_type(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Option<GateType> {
    match expected {
        SourceOptionExpectedType::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
        SourceOptionExpectedType::Tuple(elements) => Some(GateType::Tuple(
            elements
                .iter()
                .map(|element| source_option_expected_to_gate_type(element, bindings))
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::Record(fields) => Some(GateType::Record(
            fields
                .iter()
                .map(|field| {
                    Some(GateRecordField {
                        name: field.name.clone(),
                        ty: source_option_expected_to_gate_type(&field.ty, bindings)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::List(element) => Some(GateType::List(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Map { key, value } => Some(GateType::Map {
            key: Box::new(source_option_expected_to_gate_type(key, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Set(element) => Some(GateType::Set(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Signal(element) => Some(GateType::Signal(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Option(element) => Some(GateType::Option(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Result { error, value } => Some(GateType::Result {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Validation { error, value } => Some(GateType::Validation {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Named(named) => {
            let arguments = named
                .arguments
                .iter()
                .map(|argument| source_option_expected_to_gate_type(argument, bindings))
                .collect::<Option<Vec<_>>>()?;
            Some(match named.kind {
                SourceOptionNamedKind::Domain => GateType::Domain {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
                SourceOptionNamedKind::Type => GateType::OpaqueItem {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
            })
        }
        SourceOptionExpectedType::ContractParameter(parameter) => {
            bindings.parameter_gate_type(*parameter)
        }
    }
}

pub(crate) fn source_option_expected_matches_actual_type(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if !expected.is_signal_contract()
        && let SourceOptionActualType::Signal(inner) = actual {
            return source_option_expected_matches_actual_type_inner(expected, inner, bindings);
        }

    source_option_expected_matches_actual_type_inner(expected, actual, bindings)
}

pub(crate) fn source_option_expected_matches_actual_type_inner(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    match (expected, actual) {
        (SourceOptionExpectedType::ContractParameter(parameter), _) => {
            bindings.bind_or_match_actual(*parameter, actual)
        }
        (SourceOptionExpectedType::Primitive(_), SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Primitive(expected),
            SourceOptionActualType::Primitive(actual),
        ) => expected == actual,
        (SourceOptionExpectedType::Tuple(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Tuple(expected), SourceOptionActualType::Tuple(actual)) => {
            source_option_expected_args_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Record(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Record(expected), SourceOptionActualType::Record(actual)) => {
            source_option_expected_record_fields_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::List(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::List(expected), SourceOptionActualType::List(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Map { .. }, SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Map { key, value },
            SourceOptionActualType::Map {
                key: actual_key,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(key, actual_key, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Set(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Set(expected), SourceOptionActualType::Set(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Signal(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Signal(expected), SourceOptionActualType::Signal(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Option(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Option(expected), SourceOptionActualType::Option(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Result { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Result { error, value },
            SourceOptionActualType::Result {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Validation { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Validation { error, value },
            SourceOptionActualType::Validation {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Named(expected), SourceOptionActualType::Hole) => {
            let _ = expected;
            true
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::Domain {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Domain && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::OpaqueItem {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Type && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        _ => false,
    }
}

pub(crate) fn source_option_expected_args_match(
    expected: &[SourceOptionExpectedType],
    actual: &[SourceOptionActualType],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(expected, actual)| {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        })
}

pub(crate) fn source_option_expected_record_fields_match(
    expected: &[SourceOptionExpectedRecordField],
    actual: &[SourceOptionActualRecordField],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let actual_fields = actual
        .iter()
        .map(|field| (field.name.as_str(), &field.ty))
        .collect::<HashMap<_, _>>();
    expected.iter().all(|field| {
        actual_fields
            .get(field.name.as_str())
            .is_some_and(|actual| {
                source_option_expected_matches_actual_type(&field.ty, actual, bindings)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{GateType, TypeConstructorHead};
    use crate::{BuiltinType, ImportId, ImportTypeDefinition, ImportValueType};

    #[test]
    fn constructor_view_expands_transparent_import_aliases_before_matching_heads() {
        let envelope = GateType::OpaqueImport {
            import: ImportId::from_raw(7),
            name: "Envelope".into(),
            arguments: vec![GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Int,
            )))],
            definition: Some(Box::new(ImportTypeDefinition::Alias(
                ImportValueType::TypeVariable {
                    index: 0,
                    name: "A".into(),
                },
            ))),
        };

        let Some((head, arguments)) = envelope.constructor_view() else {
            panic!("transparent import alias should expose the underlying constructor view");
        };
        assert_eq!(head, TypeConstructorHead::Builtin(BuiltinType::Option));
        assert_eq!(arguments, vec![GateType::Primitive(BuiltinType::Int)]);
    }

    #[test]
    fn fits_template_accepts_transparent_import_aliases_of_builtin_carriers() {
        let envelope_option = GateType::OpaqueImport {
            import: ImportId::from_raw(7),
            name: "Envelope".into(),
            arguments: vec![GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Int,
            )))],
            definition: Some(Box::new(ImportTypeDefinition::Alias(
                ImportValueType::TypeVariable {
                    index: 0,
                    name: "A".into(),
                },
            ))),
        };
        let template = GateType::Arrow {
            parameter: Box::new(GateType::Arrow {
                parameter: Box::new(GateType::Primitive(BuiltinType::Int)),
                result: Box::new(GateType::Primitive(BuiltinType::Int)),
            }),
            result: Box::new(GateType::Arrow {
                parameter: Box::new(GateType::Option(Box::new(GateType::Primitive(
                    BuiltinType::Int,
                )))),
                result: Box::new(GateType::Option(Box::new(GateType::Primitive(
                    BuiltinType::Int,
                )))),
            }),
        };
        let actual = GateType::Arrow {
            parameter: Box::new(GateType::Arrow {
                parameter: Box::new(GateType::Primitive(BuiltinType::Int)),
                result: Box::new(GateType::Primitive(BuiltinType::Int)),
            }),
            result: Box::new(GateType::Arrow {
                parameter: Box::new(GateType::Option(Box::new(GateType::Primitive(
                    BuiltinType::Int,
                )))),
                result: Box::new(envelope_option),
            }),
        };

        assert!(actual.fits_template(&template));
    }
}
