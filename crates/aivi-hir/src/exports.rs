use crate::{
    BuiltinTerm, ExportItem, ExportResolution, ImportBindingMetadata, ImportBundleKind,
    ImportRecordField, ImportValueType, Item, ItemId, Module, ResolutionState, TypeId,
    TypeItemBody, TypeKind, TypeReference, TypeResolution,
};

/// The kind of an exported name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportedNameKind {
    Type,
    Value,
    Function,
    Signal,
    Class,
    Domain,
    SourceProvider,
    Instance,
}

/// A single exported name from a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedName {
    pub name: String,
    pub kind: ExportedNameKind,
    pub metadata: ImportBindingMetadata,
}

/// The complete set of names exported from a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportedNames(pub Vec<ExportedName>);

impl ExportedNames {
    pub fn find(&self, name: &str) -> Option<&ExportedName> {
        self.0.iter().find(|exported| exported.name == name)
    }
}

/// Extract the set of names exported from a HIR module.
///
/// Explicit `export` declarations narrow the set; if there are none, all
/// top-level named items are considered exported.
pub fn exports(module: &Module) -> ExportedNames {
    let mut names = if module
        .root_items()
        .iter()
        .any(|item_id| matches!(module.items().get(*item_id), Some(Item::Export(_))))
    {
        explicit_exported_names(module)
    } else {
        implicit_exported_names(module)
    };

    names.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| exported_kind_rank(left.kind).cmp(&exported_kind_rank(right.kind)))
    });
    ExportedNames(names)
}

fn explicit_exported_names(module: &Module) -> Vec<ExportedName> {
    let mut names = Vec::new();
    for &id in module.root_items() {
        let Some(Item::Export(export)) = module.items().get(id) else {
            continue;
        };
        let Some(exported) = export_item_to_exported_name(module, export) else {
            continue;
        };
        push_unique_exported_name(&mut names, exported);
    }
    names
}

fn implicit_exported_names(module: &Module) -> Vec<ExportedName> {
    let mut names = Vec::new();
    for &id in module.root_items() {
        if let Some(item) = module.items().get(id)
            && let Some(exported) = item_to_exported_name(module, item)
        {
            push_unique_exported_name(&mut names, exported);
        }
    }
    names
}

fn push_unique_exported_name(names: &mut Vec<ExportedName>, exported: ExportedName) {
    if names
        .iter()
        .any(|existing| existing.name == exported.name && existing.kind == exported.kind)
    {
        return;
    }
    names.push(exported);
}

fn export_item_to_exported_name(module: &Module, export: &ExportItem) -> Option<ExportedName> {
    let ResolutionState::Resolved(resolution) = export.resolution else {
        return None;
    };
    let exported_name = export.target.segments().first().text().to_owned();
    match resolution {
        ExportResolution::BuiltinType(builtin) => Some(ExportedName {
            name: exported_name,
            kind: ExportedNameKind::Type,
            metadata: ImportBindingMetadata::BuiltinType(builtin),
        }),
        ExportResolution::BuiltinTerm(builtin) => Some(ExportedName {
            name: exported_name,
            kind: ExportedNameKind::Value,
            metadata: ImportBindingMetadata::BuiltinTerm(builtin),
        }),
        ExportResolution::Item(item_id) => {
            explicit_item_exported_name(module, item_id, exported_name.as_str())
        }
    }
}

fn explicit_item_exported_name(
    module: &Module,
    item_id: ItemId,
    exported_name: &str,
) -> Option<ExportedName> {
    let item = module.items().get(item_id)?;
    let ambient = module
        .ambient_items()
        .iter()
        .any(|ambient_id| *ambient_id == item_id);
    match item {
        Item::Type(item) => {
            if item.name.text() == exported_name {
                let metadata = if ambient {
                    ImportBindingMetadata::AmbientType
                } else {
                    ImportBindingMetadata::TypeConstructor {
                        kind: aivi_typing::Kind::constructor(item.parameters.len()),
                    }
                };
                return Some(ExportedName {
                    name: exported_name.to_owned(),
                    kind: ExportedNameKind::Type,
                    metadata,
                });
            }

            let TypeItemBody::Sum(variants) = &item.body else {
                return None;
            };
            variants
                .iter()
                .any(|variant| variant.name.text() == exported_name)
                .then(|| ExportedName {
                    name: exported_name.to_owned(),
                    kind: ExportedNameKind::Value,
                    metadata: builtin_term_metadata(exported_name)
                        .unwrap_or(ImportBindingMetadata::OpaqueValue),
                })
        }
        Item::Class(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Class,
            metadata: if ambient {
                ImportBindingMetadata::AmbientType
            } else {
                ImportBindingMetadata::TypeConstructor {
                    kind: aivi_typing::Kind::constructor(item.parameters.len()),
                }
            },
        }),
        Item::Domain(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Domain,
            metadata: if ambient {
                ImportBindingMetadata::AmbientType
            } else {
                ImportBindingMetadata::TypeConstructor {
                    kind: aivi_typing::Kind::constructor(item.parameters.len()),
                }
            },
        }),
        Item::Value(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Value,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::Function(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Function,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::Signal(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Signal,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) => {
            None
        }
    }
}

fn item_to_exported_name(module: &Module, item: &Item) -> Option<ExportedName> {
    match item {
        Item::Type(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Type,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind: aivi_typing::Kind::constructor(item.parameters.len()),
            },
        }),
        Item::Value(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Value,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::Function(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Function,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::Signal(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Signal,
            metadata: exported_value_metadata(module, item.annotation),
        }),
        Item::Class(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Class,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind: aivi_typing::Kind::constructor(item.parameters.len()),
            },
        }),
        Item::Domain(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Domain,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind: aivi_typing::Kind::constructor(item.parameters.len()),
            },
        }),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) => {
            None
        }
    }
}

fn exported_value_metadata(module: &Module, annotation: Option<TypeId>) -> ImportBindingMetadata {
    annotation
        .and_then(|annotation| import_value_type(module, annotation))
        .map(|ty| ImportBindingMetadata::Value { ty })
        .unwrap_or(ImportBindingMetadata::OpaqueValue)
}

fn import_value_type(module: &Module, ty: TypeId) -> Option<ImportValueType> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Name(reference) => primitive_import_value_type(reference),
        TypeKind::Tuple(elements) => Some(ImportValueType::Tuple(
            elements
                .iter()
                .map(|element| import_value_type(module, *element))
                .collect::<Option<Vec<_>>>()?,
        )),
        TypeKind::Record(fields) => Some(ImportValueType::Record(
            fields
                .iter()
                .map(|field| {
                    Some(ImportRecordField {
                        name: field.label.text().into(),
                        ty: import_value_type(module, field.ty)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        TypeKind::Arrow { parameter, result } => Some(ImportValueType::Arrow {
            parameter: Box::new(import_value_type(module, *parameter)?),
            result: Box::new(import_value_type(module, *result)?),
        }),
        TypeKind::Apply { .. } => applied_import_value_type(module, ty),
    }
}

fn primitive_import_value_type(reference: &TypeReference) -> Option<ImportValueType> {
    let ResolutionState::Resolved(TypeResolution::Builtin(builtin)) = reference.resolution.as_ref()
    else {
        return None;
    };
    match builtin {
        crate::BuiltinType::Int
        | crate::BuiltinType::Float
        | crate::BuiltinType::Decimal
        | crate::BuiltinType::BigInt
        | crate::BuiltinType::Bool
        | crate::BuiltinType::Text
        | crate::BuiltinType::Unit
        | crate::BuiltinType::Bytes => Some(ImportValueType::Primitive(*builtin)),
        crate::BuiltinType::List
        | crate::BuiltinType::Map
        | crate::BuiltinType::Set
        | crate::BuiltinType::Option
        | crate::BuiltinType::Result
        | crate::BuiltinType::Validation
        | crate::BuiltinType::Signal
        | crate::BuiltinType::Task => None,
    }
}

fn applied_import_value_type(module: &Module, ty: TypeId) -> Option<ImportValueType> {
    let (constructor, arguments) = flatten_type_application(module, ty)?;
    match constructor {
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::List) if arguments.len() == 1 => Some(
            ImportValueType::List(Box::new(import_value_type(module, arguments[0])?)),
        ),
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Map) if arguments.len() == 2 => {
            Some(ImportValueType::Map {
                key: Box::new(import_value_type(module, arguments[0])?),
                value: Box::new(import_value_type(module, arguments[1])?),
            })
        }
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Set) if arguments.len() == 1 => Some(
            ImportValueType::Set(Box::new(import_value_type(module, arguments[0])?)),
        ),
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Option) if arguments.len() == 1 => {
            Some(ImportValueType::Option(Box::new(import_value_type(
                module,
                arguments[0],
            )?)))
        }
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Result) if arguments.len() == 2 => {
            Some(ImportValueType::Result {
                error: Box::new(import_value_type(module, arguments[0])?),
                value: Box::new(import_value_type(module, arguments[1])?),
            })
        }
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Validation)
            if arguments.len() == 2 =>
        {
            Some(ImportValueType::Validation {
                error: Box::new(import_value_type(module, arguments[0])?),
                value: Box::new(import_value_type(module, arguments[1])?),
            })
        }
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Signal) if arguments.len() == 1 => {
            Some(ImportValueType::Signal(Box::new(import_value_type(
                module,
                arguments[0],
            )?)))
        }
        ResolvedTypeConstructor::Builtin(crate::BuiltinType::Task) if arguments.len() == 2 => {
            Some(ImportValueType::Task {
                error: Box::new(import_value_type(module, arguments[0])?),
                value: Box::new(import_value_type(module, arguments[1])?),
            })
        }
        ResolvedTypeConstructor::Bundle(ImportBundleKind::BuiltinOption)
            if arguments.len() == 1 =>
        {
            Some(ImportValueType::Option(Box::new(import_value_type(
                module,
                arguments[0],
            )?)))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedTypeConstructor {
    Builtin(crate::BuiltinType),
    Bundle(ImportBundleKind),
}

fn flatten_type_application(
    module: &Module,
    ty: TypeId,
) -> Option<(ResolvedTypeConstructor, Vec<TypeId>)> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Apply { callee, arguments } => {
            let (constructor, mut flattened) = flatten_type_application(module, *callee)?;
            flattened.extend(arguments.iter().copied());
            Some((constructor, flattened))
        }
        TypeKind::Name(reference) => {
            Some((resolve_type_constructor(module, reference)?, Vec::new()))
        }
        TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
    }
}

fn resolve_type_constructor(
    module: &Module,
    reference: &TypeReference,
) -> Option<ResolvedTypeConstructor> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
            Some(ResolvedTypeConstructor::Builtin(*builtin))
        }
        ResolutionState::Resolved(TypeResolution::Import(import_id)) => match &module.imports()
            [*import_id]
            .metadata
        {
            ImportBindingMetadata::BuiltinType(builtin) => {
                Some(ResolvedTypeConstructor::Builtin(*builtin))
            }
            ImportBindingMetadata::Bundle(bundle) => Some(ResolvedTypeConstructor::Bundle(*bundle)),
            ImportBindingMetadata::Unknown
            | ImportBindingMetadata::Value { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType => None,
        },
        ResolutionState::Resolved(TypeResolution::Item(_))
        | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
        | ResolutionState::Unresolved => None,
    }
}

fn exported_kind_rank(kind: ExportedNameKind) -> u8 {
    match kind {
        ExportedNameKind::Type => 0,
        ExportedNameKind::Value => 1,
        ExportedNameKind::Function => 2,
        ExportedNameKind::Signal => 3,
        ExportedNameKind::Class => 4,
        ExportedNameKind::Domain => 5,
        ExportedNameKind::SourceProvider => 6,
        ExportedNameKind::Instance => 7,
    }
}

fn builtin_term_metadata(name: &str) -> Option<ImportBindingMetadata> {
    match name {
        "True" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::True)),
        "False" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::False)),
        "None" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::None)),
        "Some" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Some)),
        "Ok" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Ok)),
        "Err" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Err)),
        "Valid" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Valid)),
        "Invalid" => Some(ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Invalid)),
        _ => None,
    }
}
