#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateType {
    Primitive(BuiltinType),
    TypeParameter {
        parameter: TypeParameterId,
        name: String,
    },
    Tuple(Vec<GateType>),
    Record(Vec<GateRecordField>),
    Arrow {
        parameter: Box<GateType>,
        result: Box<GateType>,
    },
    List(Box<GateType>),
    Map {
        key: Box<GateType>,
        value: Box<GateType>,
    },
    Set(Box<GateType>),
    Option(Box<GateType>),
    Result {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Validation {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Signal(Box<GateType>),
    Task {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
    /// An imported type constructor or domain from another module.
    OpaqueImport {
        import: ImportId,
        name: String,
        arguments: Vec<GateType>,
        definition: Option<Box<ImportTypeDefinition>>,
    },
}

impl GateType {
    pub(crate) fn is_bool(&self) -> bool {
        matches!(self, Self::Primitive(BuiltinType::Bool))
    }

    pub(crate) fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn gate_carrier(&self) -> GateCarrier {
        match self {
            Self::Signal(_) => GateCarrier::Signal,
            _ => GateCarrier::Ordinary,
        }
    }

    pub(crate) fn gate_payload(&self) -> &Self {
        match self {
            Self::Signal(inner) => inner,
            other => other,
        }
    }

    pub(crate) fn fanout_carrier(&self) -> Option<FanoutCarrier> {
        match self {
            Self::List(_) => Some(FanoutCarrier::Ordinary),
            Self::Signal(inner) if matches!(inner.as_ref(), Self::List(_)) => {
                Some(FanoutCarrier::Signal)
            }
            _ => None,
        }
    }

    pub(crate) fn fanout_element(&self) -> Option<&Self> {
        match self {
            Self::List(element) => Some(element),
            Self::Signal(inner) => match inner.as_ref() {
                Self::List(element) => Some(element),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn recurrence_target_evidence(&self) -> Option<RecurrenceTargetEvidence> {
        match self {
            Self::Signal(_) => Some(RecurrenceTargetEvidence::ExplicitSignalAnnotation),
            Self::Task { .. } => Some(RecurrenceTargetEvidence::ExplicitTaskAnnotation),
            _ => None,
        }
    }

    /// Extract the canonical name and arguments from a user-defined type variant
    /// (Domain, OpaqueItem, OpaqueImport). Returns None for builtin/primitive types.
    fn named_type_parts(&self) -> Option<(&str, &[GateType])> {
        match self {
            Self::Domain {
                name, arguments, ..
            }
            | Self::OpaqueItem {
                name, arguments, ..
            }
            | Self::OpaqueImport {
                name, arguments, ..
            } => Some((name.as_str(), arguments.as_slice())),
            _ => None,
        }
    }

    pub(crate) fn same_shape(&self, other: &Self) -> bool {
        let mut left_to_right = HashMap::new();
        let mut right_to_left = HashMap::new();
        Self::same_shape_inner(self, other, &mut left_to_right, &mut right_to_left)
    }

    /// Substitute every occurrence of `param` with `replacement` throughout this type.
    pub(crate) fn substitute_type_parameter(
        &self,
        param: TypeParameterId,
        replacement: &GateType,
    ) -> GateType {
        self.substitute_type_parameters(&HashMap::from([(param, replacement.clone())]))
    }

    /// Substitute multiple type parameters simultaneously using the given map.
    pub(crate) fn substitute_type_parameters(
        &self,
        subs: &HashMap<TypeParameterId, GateType>,
    ) -> GateType {
        if subs.is_empty() {
            return self.clone();
        }
        match self {
            Self::TypeParameter { parameter, .. } => {
                subs.get(parameter).cloned().unwrap_or_else(|| self.clone())
            }
            Self::Primitive(_) => self.clone(),
            Self::Arrow { parameter, result } => Self::Arrow {
                parameter: Box::new(parameter.substitute_type_parameters(subs)),
                result: Box::new(result.substitute_type_parameters(subs)),
            },
            Self::List(element) => Self::List(Box::new(element.substitute_type_parameters(subs))),
            Self::Option(element) => {
                Self::Option(Box::new(element.substitute_type_parameters(subs)))
            }
            Self::Signal(element) => {
                Self::Signal(Box::new(element.substitute_type_parameters(subs)))
            }
            Self::Tuple(elements) => Self::Tuple(
                elements
                    .iter()
                    .map(|e| e.substitute_type_parameters(subs))
                    .collect(),
            ),
            Self::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|f| GateRecordField {
                        name: f.name.clone(),
                        ty: f.ty.substitute_type_parameters(subs),
                    })
                    .collect(),
            ),
            Self::Map { key, value } => Self::Map {
                key: Box::new(key.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Set(element) => Self::Set(Box::new(element.substitute_type_parameters(subs))),
            Self::Result { error, value } => Self::Result {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Validation { error, value } => Self::Validation {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Task { error, value } => Self::Task {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Domain {
                item,
                name,
                arguments,
            } => Self::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
            },
            Self::OpaqueItem {
                item,
                name,
                arguments,
            } => Self::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
            },
            Self::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Self::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
                definition: definition.clone(),
            },
        }
    }

    /// Returns true when `self` (a concrete type) is a valid instantiation of `template`, treating
    /// any `TypeParameter` in `template` as an unconstrained wildcard.
    pub(crate) fn has_type_params(&self) -> bool {
        match self {
            Self::TypeParameter { .. } => true,
            Self::Primitive(_) => false,
            Self::Arrow { parameter, result } => {
                parameter.has_type_params() || result.has_type_params()
            }
            Self::List(e) | Self::Option(e) | Self::Signal(e) | Self::Set(e) => e.has_type_params(),
            Self::Tuple(elements) => elements.iter().any(|e| e.has_type_params()),
            Self::Record(fields) => fields.iter().any(|f| f.ty.has_type_params()),
            Self::Map { key, value } => key.has_type_params() || value.has_type_params(),
            Self::Result { error, value }
            | Self::Validation { error, value }
            | Self::Task { error, value } => error.has_type_params() || value.has_type_params(),
            Self::Domain { arguments, .. }
            | Self::OpaqueItem { arguments, .. }
            | Self::OpaqueImport { arguments, .. } => arguments.iter().any(|a| a.has_type_params()),
        }
    }

    pub(crate) fn fits_template(&self, template: &Self) -> bool {
        if let Some(expanded_self) = self.expand_transparent_import_alias() {
            return expanded_self.fits_template(template);
        }
        if let Some(expanded_template) = template.expand_transparent_import_alias() {
            return self.fits_template(&expanded_template);
        }
        match template {
            Self::TypeParameter { .. } => true,
            Self::Primitive(_) => self == template,
            Self::Arrow {
                parameter: tp,
                result: tr,
            } => match self {
                Self::Arrow {
                    parameter: sp,
                    result: sr,
                } => sp.fits_template(tp) && sr.fits_template(tr),
                _ => false,
            },
            Self::List(te) => match self {
                Self::List(se) => se.fits_template(te),
                _ => false,
            },
            Self::Option(te) => match self {
                Self::Option(se) => se.fits_template(te),
                _ => false,
            },
            Self::Signal(te) => match self {
                Self::Signal(se) => se.fits_template(te),
                _ => false,
            },
            Self::Set(te) => match self {
                Self::Set(se) => se.fits_template(te),
                _ => false,
            },
            Self::Tuple(tes) => match self {
                Self::Tuple(ses) => {
                    ses.len() == tes.len()
                        && ses.iter().zip(tes.iter()).all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::Record(tfields) => match self {
                Self::Record(sfields) => {
                    sfields.len() == tfields.len()
                        && sfields
                            .iter()
                            .zip(tfields.iter())
                            .all(|(s, t)| s.name == t.name && s.ty.fits_template(&t.ty))
                }
                _ => false,
            },
            Self::Map { key: tk, value: tv } => match self {
                Self::Map { key: sk, value: sv } => sk.fits_template(tk) && sv.fits_template(tv),
                _ => false,
            },
            Self::Result {
                error: te,
                value: tv,
            } => match self {
                Self::Result {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Validation {
                error: te,
                value: tv,
            } => match self {
                Self::Validation {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Task {
                error: te,
                value: tv,
            } => match self {
                Self::Task {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Domain {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::OpaqueItem {
                arguments: targs,
                name: tname,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::OpaqueImport {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
        }
    }

    /// Structurally match `self` (concrete) against `template` (may contain TypeParameter nodes),
    /// collecting the bindings.  Returns `true` when matching succeeds and all TypeParameter
    /// nodes receive consistent bindings.
    pub(crate) fn unify_type_params(
        &self,
        template: &Self,
        bindings: &mut HashMap<TypeParameterId, GateType>,
    ) -> bool {
        match template {
            Self::TypeParameter { parameter, .. } => match bindings.get(parameter) {
                Some(existing) => existing.same_shape(self),
                None => {
                    bindings.insert(*parameter, self.clone());
                    true
                }
            },
            Self::Primitive(_) => self == template,
            Self::Arrow {
                parameter: tp,
                result: tr,
            } => match self {
                Self::Arrow {
                    parameter: sp,
                    result: sr,
                } => sp.unify_type_params(tp, bindings) && sr.unify_type_params(tr, bindings),
                _ => false,
            },
            Self::List(te) => match self {
                Self::List(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Option(te) => match self {
                Self::Option(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Signal(te) => match self {
                Self::Signal(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Set(te) => match self {
                Self::Set(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Tuple(tes) => match self {
                Self::Tuple(ses) => {
                    ses.len() == tes.len()
                        && ses
                            .iter()
                            .zip(tes.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::Record(tfields) => match self {
                Self::Record(sfields) => {
                    sfields.len() == tfields.len()
                        && sfields.iter().zip(tfields.iter()).all(|(s, t)| {
                            s.name == t.name && s.ty.unify_type_params(&t.ty, bindings)
                        })
                }
                _ => false,
            },
            Self::Map { key: tk, value: tv } => match self {
                Self::Map { key: sk, value: sv } => {
                    sk.unify_type_params(tk, bindings) && sv.unify_type_params(tv, bindings)
                }
                _ => false,
            },
            Self::Result {
                error: te,
                value: tv,
            } => match self {
                Self::Result {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Validation {
                error: te,
                value: tv,
            } => match self {
                Self::Validation {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Task {
                error: te,
                value: tv,
            } => match self {
                Self::Task {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Domain {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::OpaqueItem {
                arguments: targs,
                name: tname,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::OpaqueImport {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
        }
    }

    /// Expand a transparent imported type alias into a `GateType` by substituting
    /// the provided type arguments for `TypeVariable` placeholders.  Returns `None`
    /// when the alias body contains a `Named` reference that requires module context
    /// to resolve; callers should treat `None` as "cannot expand, stay opaque".
    fn expand_import_alias_type(
        alias: &ImportValueType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        match alias {
            ImportValueType::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
            ImportValueType::TypeVariable { index, .. } => arguments.get(*index).cloned(),
            ImportValueType::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(Self::expand_import_alias_type(parameter, arguments)?),
                result: Box::new(Self::expand_import_alias_type(result, arguments)?),
            }),
            ImportValueType::Tuple(elements) => {
                let lowered: Option<Vec<_>> = elements
                    .iter()
                    .map(|e| Self::expand_import_alias_type(e, arguments))
                    .collect();
                Some(GateType::Tuple(lowered?))
            }
            ImportValueType::List(element) => Some(GateType::List(Box::new(
                Self::expand_import_alias_type(element, arguments)?,
            ))),
            ImportValueType::Map { key, value } => Some(GateType::Map {
                key: Box::new(Self::expand_import_alias_type(key, arguments)?),
                value: Box::new(Self::expand_import_alias_type(value, arguments)?),
            }),
            ImportValueType::Set(element) => Some(GateType::Set(Box::new(
                Self::expand_import_alias_type(element, arguments)?,
            ))),
            ImportValueType::Option(element) => Some(GateType::Option(Box::new(
                Self::expand_import_alias_type(element, arguments)?,
            ))),
            ImportValueType::Result { error, value } => Some(GateType::Result {
                error: Box::new(Self::expand_import_alias_type(error, arguments)?),
                value: Box::new(Self::expand_import_alias_type(value, arguments)?),
            }),
            ImportValueType::Validation { error, value } => Some(GateType::Validation {
                error: Box::new(Self::expand_import_alias_type(error, arguments)?),
                value: Box::new(Self::expand_import_alias_type(value, arguments)?),
            }),
            ImportValueType::Signal(element) => Some(GateType::Signal(Box::new(
                Self::expand_import_alias_type(element, arguments)?,
            ))),
            ImportValueType::Task { error, value } => Some(GateType::Task {
                error: Box::new(Self::expand_import_alias_type(error, arguments)?),
                value: Box::new(Self::expand_import_alias_type(value, arguments)?),
            }),
            ImportValueType::Record(fields) => {
                let lowered: Option<Vec<_>> = fields
                    .iter()
                    .map(|f| {
                        Self::expand_import_alias_type(&f.ty, arguments).map(|ty| GateRecordField {
                            name: f.name.to_string(),
                            ty,
                        })
                    })
                    .collect();
                Some(GateType::Record(lowered?))
            }
            // Named references: produce a sentinel OpaqueImport keyed by name so that
            // same_shape_inner can match via named_type_parts even without a real ImportId.
            ImportValueType::Named {
                type_name,
                arguments: type_args,
                ..
            } => {
                let lowered_args: Vec<GateType> = type_args
                    .iter()
                    .map(|a| {
                        Self::expand_import_alias_type(a, arguments).unwrap_or_else(|| {
                            GateType::OpaqueImport {
                                import: ImportId::from_raw(u32::MAX),
                                name: String::new(),
                                arguments: Vec::new(),
                                definition: None,
                            }
                        })
                    })
                    .collect();
                Some(GateType::OpaqueImport {
                    import: ImportId::from_raw(u32::MAX),
                    name: type_name.clone(),
                    arguments: lowered_args,
                    definition: None,
                })
            }
        }
    }

    pub(crate) fn same_shape_inner(
        left: &Self,
        right: &Self,
        left_to_right: &mut HashMap<TypeParameterId, TypeParameterId>,
        right_to_left: &mut HashMap<TypeParameterId, TypeParameterId>,
    ) -> bool {
        match (left, right) {
            (Self::Primitive(left), Self::Primitive(right)) => left == right,
            (
                Self::TypeParameter {
                    parameter: left_parameter,
                    ..
                },
                Self::TypeParameter {
                    parameter: right_parameter,
                    ..
                },
            ) => match (
                left_to_right.get(left_parameter),
                right_to_left.get(right_parameter),
            ) {
                (Some(mapped_right), Some(mapped_left)) => {
                    mapped_right == right_parameter && mapped_left == left_parameter
                }
                (None, None) => {
                    left_to_right.insert(*left_parameter, *right_parameter);
                    right_to_left.insert(*right_parameter, *left_parameter);
                    true
                }
                _ => false,
            },
            (Self::Tuple(left), Self::Tuple(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        Self::same_shape_inner(left, right, left_to_right, right_to_left)
                    })
            }
            (Self::Record(left), Self::Record(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        left.name == right.name
                            && Self::same_shape_inner(
                                &left.ty,
                                &right.ty,
                                left_to_right,
                                right_to_left,
                            )
                    })
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
            ) => {
                Self::same_shape_inner(
                    left_parameter,
                    right_parameter,
                    left_to_right,
                    right_to_left,
                ) && Self::same_shape_inner(left_result, right_result, left_to_right, right_to_left)
            }
            (Self::List(left), Self::List(right))
            | (Self::Set(left), Self::Set(right))
            | (Self::Option(left), Self::Option(right))
            | (Self::Signal(left), Self::Signal(right)) => {
                Self::same_shape_inner(left, right, left_to_right, right_to_left)
            }
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_key, right_key, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
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
            )
            | (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_error, right_error, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
            }
            (
                Self::Domain {
                    item: left_item,
                    name: left_name,
                    arguments: left_arguments,
                },
                Self::Domain {
                    item: right_item,
                    name: right_name,
                    arguments: right_arguments,
                },
            ) => {
                (left_item == right_item || left_name == right_name)
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            (
                Self::OpaqueItem {
                    item: left_item,
                    arguments: left_arguments,
                    ..
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) => {
                left_item == right_item
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            (
                Self::OpaqueImport {
                    import: left_import,
                    name: left_name,
                    arguments: left_arguments,
                    ..
                },
                Self::OpaqueImport {
                    import: right_import,
                    name: right_name,
                    arguments: right_arguments,
                    ..
                },
            ) => {
                // Allow matching by name when either side has a sentinel import ID
                // (u32::MAX), which is used when expanding type aliases without
                // full module context (e.g. Named references in expand_import_alias_type).
                let sentinel = ImportId::from_raw(u32::MAX);
                let ids_match = left_import == right_import
                    || *left_import == sentinel
                    || *right_import == sentinel;
                ids_match
                    && left_name == right_name
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            // Cross-variant name-based equivalence: Domain, OpaqueItem, and
            // OpaqueImport all represent the same logical type when their canonical
            // names and argument shapes agree.  This covers ambient-prelude types
            // versus stdlib-imported types across all variant combinations.
            _ => {
                match (left.named_type_parts(), right.named_type_parts()) {
                    (Some((ln, la)), Some((rn, ra))) => {
                        if ln == rn
                            && la.len() == ra.len()
                            && la.iter().zip(ra.iter()).all(|(l, r)| {
                                Self::same_shape_inner(l, r, left_to_right, right_to_left)
                            })
                        {
                            return true;
                        }
                    }
                    _ => {}
                }
                // Expand transparent imported type aliases (e.g. `type Envelope A = A`)
                // so that `Envelope Text` is recognised as the same shape as `Text`.
                if let Self::OpaqueImport {
                    arguments,
                    definition: Some(def),
                    ..
                } = left
                {
                    if let ImportTypeDefinition::Alias(alias) = def.as_ref() {
                        if let Some(expanded) = Self::expand_import_alias_type(alias, arguments) {
                            return Self::same_shape_inner(
                                &expanded,
                                right,
                                left_to_right,
                                right_to_left,
                            );
                        }
                    }
                }
                if let Self::OpaqueImport {
                    arguments,
                    definition: Some(def),
                    ..
                } = right
                {
                    if let ImportTypeDefinition::Alias(alias) = def.as_ref() {
                        if let Some(expanded) = Self::expand_import_alias_type(alias, arguments) {
                            return Self::same_shape_inner(
                                left,
                                &expanded,
                                left_to_right,
                                right_to_left,
                            );
                        }
                    }
                }
                false
            }
        }
    }

    pub(crate) fn constructor_view(&self) -> Option<(TypeConstructorHead, Vec<GateType>)> {
        if let Some(expanded) = self.expand_transparent_import_alias()
            && let Some(view) = expanded.constructor_view()
        {
            return Some(view);
        }
        match self {
            Self::List(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::List),
                vec![element.as_ref().clone()],
            )),
            Self::Map { key, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Map),
                vec![key.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Set(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Set),
                vec![element.as_ref().clone()],
            )),
            Self::Option(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Option),
                vec![element.as_ref().clone()],
            )),
            Self::Result { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Result),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Validation { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Validation),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Signal(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Signal),
                vec![element.as_ref().clone()],
            )),
            Self::Task { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Task),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Domain {
                item, arguments, ..
            }
            | Self::OpaqueItem {
                item, arguments, ..
            } => Some((TypeConstructorHead::Item(*item), arguments.clone())),
            Self::OpaqueImport {
                import, arguments, ..
            } => Some((TypeConstructorHead::Import(*import), arguments.clone())),
            Self::Primitive(_)
            | Self::TypeParameter { .. }
            | Self::Tuple(_)
            | Self::Record(_)
            | Self::Arrow { .. } => None,
        }
    }

    fn expand_transparent_import_alias(&self) -> Option<GateType> {
        let Self::OpaqueImport {
            arguments,
            definition: Some(definition),
            ..
        } = self
        else {
            return None;
        };
        let ImportTypeDefinition::Alias(alias) = definition.as_ref() else {
            return None;
        };
        Self::expand_import_alias_type(alias, arguments)
    }
}

impl fmt::Display for GateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateType::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            GateType::TypeParameter { name, .. } => write!(f, "{name}"),
            GateType::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            GateType::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            GateType::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            GateType::List(element) => write!(f, "List {element}"),
            GateType::Map { key, value } => write!(f, "Map {key} {value}"),
            GateType::Set(element) => write!(f, "Set {element}"),
            GateType::Option(element) => write!(f, "Option {element}"),
            GateType::Result { error, value } => write!(f, "Result {error} {value}"),
            GateType::Validation { error, value } => {
                write!(f, "Validation {error} {value}")
            }
            GateType::Signal(element) => write!(f, "Signal {element}"),
            GateType::Task { error, value } => write!(f, "Task {error} {value}"),
            GateType::Domain {
                name, arguments, ..
            }
            | GateType::OpaqueItem {
                name, arguments, ..
            }
            | GateType::OpaqueImport {
                name, arguments, ..
            } => {
                write!(f, "{name}")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                Ok(())
            }
        }
    }
}
