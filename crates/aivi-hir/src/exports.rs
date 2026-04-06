use std::collections::HashMap;

use crate::{
    BuiltinTerm, DecoratorPayload, DeprecatedDecorator, DeprecationNotice, DomainMemberKind,
    ExportItem, ExportResolution, ImportBindingMetadata, ImportBundleKind, ImportId,
    ImportRecordField, ImportValueType, ImportedDomainLiteralSuffix, Item, ItemId, Module,
    RecordExpr, ResolutionState, TypeId, TypeItemBody, TypeKind, TypeParameterId, TypeReference,
    TypeResolution,
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
    pub callable_type: Option<ImportValueType>,
    pub deprecation: Option<DeprecationNotice>,
}

/// The complete set of names exported from a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportedNames {
    pub names: Vec<ExportedName>,
    pub instances: Vec<ExportedInstanceDeclaration>,
}

impl ExportedNames {
    pub fn find(&self, name: &str) -> Option<&ExportedName> {
        self.names.iter().find(|exported| exported.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ExportedName> {
        self.names.iter()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
}

/// A class instance declaration exported from a module, carrying enough
/// metadata for the importing module to resolve cross-module class instances.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedInstanceDeclaration {
    pub class_name: Box<str>,
    pub subject: Box<str>,
    pub members: Vec<ExportedInstanceMember>,
}

/// One member of an exported class instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedInstanceMember {
    pub name: Box<str>,
    pub ty: ImportValueType,
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
    let instances = collect_instance_declarations(module);
    ExportedNames { names, instances }
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
        let Some(item) = module.items().get(id) else {
            continue;
        };
        if let Some(exported) = item_to_exported_name(module, item) {
            push_unique_exported_name(&mut names, exported);
        }
        // For sum types, also export each constructor individually so that
        // `use module (ConstructorName)` works for modules using implicit exports.
        if let Item::Type(type_item) = item {
            if let TypeItemBody::Sum(variants) = &type_item.body {
                let deprecation = item_deprecation_notice(module, item);
                for variant in variants.iter() {
                    let name = variant.name.text().to_owned();
                    let metadata = builtin_term_metadata(&name).unwrap_or_else(|| {
                        let owner_type_name: String = type_item.name.text().into();
                        if variant.fields.is_empty() {
                            ImportBindingMetadata::Value {
                                ty: ImportValueType::Named {
                                    type_name: owner_type_name,
                                    arguments: Vec::new(),
                                },
                            }
                        } else {
                            let result = ImportValueType::Named {
                                type_name: owner_type_name,
                                arguments: Vec::new(),
                            };
                            let ty = variant.fields.iter().rev().fold(result, |acc, field| {
                                let param_ty = import_value_type(module, field.ty).unwrap_or(
                                    ImportValueType::Named {
                                        type_name: "Unknown".into(),
                                        arguments: Vec::new(),
                                    },
                                );
                                ImportValueType::Arrow {
                                    parameter: Box::new(param_ty),
                                    result: Box::new(acc),
                                }
                            });
                            ImportBindingMetadata::Value { ty }
                        }
                    });
                    push_unique_exported_name(
                        &mut names,
                        ExportedName {
                            name,
                            kind: ExportedNameKind::Value,
                            metadata,
                            callable_type: None,
                            deprecation: deprecation.clone(),
                        },
                    );
                }
            }
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
            callable_type: None,
            deprecation: None,
        }),
        ExportResolution::BuiltinTerm(builtin) => Some(ExportedName {
            name: exported_name,
            kind: ExportedNameKind::Value,
            metadata: ImportBindingMetadata::BuiltinTerm(builtin),
            callable_type: None,
            deprecation: None,
        }),
        ExportResolution::Item(item_id) => {
            explicit_item_exported_name(module, item_id, exported_name.as_str())
        }
        ExportResolution::Import(import_id) => {
            re_exported_import_name(module, import_id, exported_name.as_str())
        }
    }
}

fn re_exported_import_name(
    module: &Module,
    import_id: ImportId,
    exported_name: &str,
) -> Option<ExportedName> {
    let import = module.imports().get(import_id)?;
    let kind = match &import.metadata {
        ImportBindingMetadata::TypeConstructor { .. }
        | ImportBindingMetadata::Domain { .. }
        | ImportBindingMetadata::BuiltinType(_)
        | ImportBindingMetadata::AmbientType => ExportedNameKind::Type,
        _ => ExportedNameKind::Value,
    };
    Some(ExportedName {
        name: exported_name.to_owned(),
        kind,
        metadata: import.metadata.clone(),
        callable_type: import.callable_type.clone(),
        deprecation: import.deprecation.clone(),
    })
}

fn explicit_item_exported_name(
    module: &Module,
    item_id: ItemId,
    exported_name: &str,
) -> Option<ExportedName> {
    let item = module.items().get(item_id)?;
    if item_has_test_decorator(module, item) {
        return None;
    }
    let ambient = module
        .ambient_items()
        .iter()
        .any(|ambient_id| *ambient_id == item_id);
    let deprecation = item_deprecation_notice(module, item);
    match item {
        Item::Type(item) => {
            if item.name.text() == exported_name {
                let metadata = if ambient {
                    ImportBindingMetadata::AmbientType
                } else {
                    let fields = extract_type_record_fields(module, item);
                    ImportBindingMetadata::TypeConstructor {
                        kind: aivi_typing::Kind::constructor(item.parameters.len()),
                        fields,
                    }
                };
                return Some(ExportedName {
                    name: exported_name.to_owned(),
                    kind: ExportedNameKind::Type,
                    metadata,
                    callable_type: None,
                    deprecation,
                });
            }

            let TypeItemBody::Sum(variants) = &item.body else {
                return None;
            };
            variants
                .iter()
                .find(|variant| variant.name.text() == exported_name)
                .map(|variant| {
                    let metadata = builtin_term_metadata(exported_name)
                        .unwrap_or_else(|| {
                            // For non-builtin sum constructors, store the owner type name so
                            // that `import_value_type` can return a non-None GateType for them.
                            // Zero-arg constructors have no Arrow wrapper; constructors with
                            // fields wrap the field types in Arrow chains.
                            let owner_type_name: String = item.name.text().into();
                            if variant.fields.is_empty() {
                                ImportBindingMetadata::Value {
                                    ty: ImportValueType::Named {
                                        type_name: owner_type_name,
                                        arguments: Vec::new(),
                                    },
                                }
                            } else {
                                // Multi-field constructors: build Arrow chain over field types,
                                // returning the owner Named type.
                                let result = ImportValueType::Named {
                                    type_name: owner_type_name,
                                    arguments: Vec::new(),
                                };
                                let ty = variant.fields.iter().rev().fold(result, |acc, field| {
                                    let param_ty =
                                        import_value_type(module, field.ty).unwrap_or(
                                            ImportValueType::Named {
                                                type_name: "Unknown".into(),
                                                arguments: Vec::new(),
                                            },
                                        );
                                    ImportValueType::Arrow {
                                        parameter: Box::new(param_ty),
                                        result: Box::new(acc),
                                    }
                                });
                                ImportBindingMetadata::Value { ty }
                            }
                        });
                    ExportedName {
                        name: exported_name.to_owned(),
                        kind: ExportedNameKind::Value,
                        metadata,
                        callable_type: None,
                        deprecation,
                    }
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
                    fields: None,
                }
            },
            callable_type: None,
            deprecation,
        }),
        Item::Domain(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Domain,
            metadata: if ambient {
                ImportBindingMetadata::AmbientType
            } else {
                let literal_suffixes = item
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        (m.kind == DomainMemberKind::Literal
                            && m.name.text().chars().count() >= 2)
                            .then(|| ImportedDomainLiteralSuffix {
                                name: m.name.text().into(),
                                member_index: i,
                            })
                    })
                    .collect();
                ImportBindingMetadata::Domain {
                    kind: aivi_typing::Kind::constructor(item.parameters.len()),
                    literal_suffixes,
                    carrier: import_value_type(module, item.carrier),
                }
            },
            callable_type: None,
            deprecation,
        }),
        Item::Value(item) => (item.name.text() == exported_name).then(|| ExportedName {
            name: exported_name.to_owned(),
            kind: ExportedNameKind::Value,
            metadata: exported_value_metadata(module, item.annotation),
            callable_type: None,
            deprecation,
        }),
        Item::Function(item) => (item.name.text() == exported_name).then(|| {
            let callable_type = exported_function_type(module, item);
            let metadata = match &callable_type {
                Some(ty) => ImportBindingMetadata::Value { ty: ty.clone() },
                None => ImportBindingMetadata::OpaqueValue,
            };
            ExportedName {
                name: exported_name.to_owned(),
                kind: ExportedNameKind::Function,
                metadata,
                callable_type,
                deprecation,
            }
        }),
        Item::Signal(item) => (item.name.text() == exported_name
            && !item.is_source_capability_handle)
            .then(|| ExportedName {
                name: exported_name.to_owned(),
                kind: ExportedNameKind::Signal,
                metadata: exported_value_metadata(module, item.annotation),
                callable_type: None,
                deprecation,
            }),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::Hoist(_) => {
            None
        }
    }
}

fn item_to_exported_name(module: &Module, item: &Item) -> Option<ExportedName> {
    if item_has_test_decorator(module, item) {
        return None;
    }
    let deprecation = item_deprecation_notice(module, item);
    match item {
        Item::Type(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Type,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind: aivi_typing::Kind::constructor(item.parameters.len()),
                fields: extract_type_record_fields(module, item),
            },
            callable_type: None,
            deprecation,
        }),
        Item::Value(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Value,
            metadata: exported_value_metadata(module, item.annotation),
            callable_type: None,
            deprecation,
        }),
        Item::Function(item) => {
            let callable_type = exported_function_type(module, item);
            let metadata = match &callable_type {
                Some(ty) => ImportBindingMetadata::Value { ty: ty.clone() },
                None => ImportBindingMetadata::OpaqueValue,
            };
            Some(ExportedName {
                name: item.name.text().to_owned(),
                kind: ExportedNameKind::Function,
                metadata,
                callable_type,
                deprecation,
            })
        }
        Item::Signal(item) => (!item.is_source_capability_handle).then(|| ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Signal,
            metadata: exported_value_metadata(module, item.annotation),
            callable_type: None,
            deprecation,
        }),
        Item::Class(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Class,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind: aivi_typing::Kind::constructor(item.parameters.len()),
                fields: None,
            },
            callable_type: None,
            deprecation,
        }),
        Item::Domain(item) => Some(ExportedName {
            name: item.name.text().to_owned(),
            kind: ExportedNameKind::Domain,
            metadata: {
                let literal_suffixes = item
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        (m.kind == DomainMemberKind::Literal
                            && m.name.text().chars().count() >= 2)
                            .then(|| ImportedDomainLiteralSuffix {
                                name: m.name.text().into(),
                                member_index: i,
                            })
                    })
                    .collect();
                ImportBindingMetadata::Domain {
                    kind: aivi_typing::Kind::constructor(item.parameters.len()),
                    literal_suffixes,
                    carrier: import_value_type(module, item.carrier),
                }
            },
            callable_type: None,
            deprecation,
        }),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::Hoist(_) => {
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

/// Collect all instance declarations from a module for cross-module instance resolution.
fn collect_instance_declarations(module: &Module) -> Vec<ExportedInstanceDeclaration> {
    let mut declarations = Vec::new();
    for &item_id in module.root_items() {
        let Some(Item::Instance(instance)) = module.items().get(item_id) else {
            continue;
        };
        let class_name: Box<str> = instance
            .class
            .path
            .segments()
            .last()
            .text()
            .into();
        let Some(subject_type_id) = instance.arguments.iter().next().copied() else {
            continue;
        };
        let Some(subject) = type_label_for_export(module, subject_type_id) else {
            continue;
        };
        let members = instance
            .members
            .iter()
            .filter_map(|member| {
                let ty = member
                    .annotation
                    .and_then(|annotation| import_value_type(module, annotation))
                    .or_else(|| exported_instance_member_type(module, member))?;
                Some(ExportedInstanceMember {
                    name: member.name.text().into(),
                    ty,
                })
            })
            .collect();
        declarations.push(ExportedInstanceDeclaration {
            class_name,
            subject,
            members,
        });
    }
    declarations
}

/// Build a portable type for an instance member from its parameters and annotation,
/// falling back to a function type inferred from parameter annotations and body annotation.
fn exported_instance_member_type(
    module: &Module,
    member: &crate::InstanceMember,
) -> Option<ImportValueType> {
    if member.parameters.is_empty() {
        return None;
    }
    let type_param_map: HashMap<TypeParameterId, usize> = HashMap::new();
    let result = member
        .annotation
        .and_then(|annotation| poly_import_value_type(module, annotation, &type_param_map))?;
    let mut ty = result;
    for param in member.parameters.iter().rev() {
        let param_ty = param
            .annotation
            .and_then(|annotation| poly_import_value_type(module, annotation, &type_param_map))?;
        ty = ImportValueType::Arrow {
            parameter: Box::new(param_ty),
            result: Box::new(ty),
        };
    }
    Some(ty)
}

/// Extract a string label for a type, used for cross-module instance subject matching.
fn type_label_for_export(module: &Module, ty: TypeId) -> Option<Box<str>> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Name(reference) => {
            Some(reference.path.segments().last().text().into())
        }
        TypeKind::Apply { callee, .. } => type_label_for_export(module, *callee),
        _ => None,
    }
}

fn exported_function_type(module: &Module, item: &crate::FunctionItem) -> Option<ImportValueType> {
    if !item.context.is_empty() {
        return None;
    }
    // Build a mapping from TypeParameterId → index for polymorphic functions.
    let type_param_map: HashMap<TypeParameterId, usize> = item
        .type_parameters
        .iter()
        .enumerate()
        .map(|(i, &p)| (p, i))
        .collect();
    let Some(mut result) = item
        .annotation
        .and_then(|annotation| poly_import_value_type(module, annotation, &type_param_map))
    else {
        return None;
    };
    for parameter in item.parameters.iter().rev() {
        let Some(parameter_ty) = parameter
            .annotation
            .and_then(|annotation| poly_import_value_type(module, annotation, &type_param_map))
        else {
            return None;
        };
        result = ImportValueType::Arrow {
            parameter: Box::new(parameter_ty),
            result: Box::new(result),
        };
    }
    Some(result)
}

fn item_has_test_decorator(module: &Module, item: &Item) -> bool {
    item.decorators().iter().any(|decorator_id| {
        module
            .decorators()
            .get(*decorator_id)
            .is_some_and(|decorator| matches!(decorator.payload, DecoratorPayload::Test(_)))
    })
}

fn item_deprecation_notice(module: &Module, item: &Item) -> Option<DeprecationNotice> {
    item.decorators().iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        let DecoratorPayload::Deprecated(deprecated) = &decorator.payload else {
            return None;
        };
        Some(deprecation_notice(module, deprecated))
    })
}

fn deprecation_notice(module: &Module, deprecated: &DeprecatedDecorator) -> DeprecationNotice {
    DeprecationNotice {
        message: deprecated
            .message
            .and_then(|message| module.expr_static_text(message)),
        replacement: deprecated.options.and_then(|options| {
            let expr = module.exprs().get(options)?;
            let crate::ExprKind::Record(RecordExpr { fields }) = &expr.kind else {
                return None;
            };
            fields
                .iter()
                .find(|field| field.label.text() == "replacement")
                .and_then(|field| module.expr_static_text(field.value))
        }),
    }
}

pub(crate) fn import_value_type(module: &Module, ty: TypeId) -> Option<ImportValueType> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Name(reference) => {
            // First try builtins.
            if let Some(prim) = primitive_import_value_type(reference) {
                return Some(prim);
            }
            // For user-defined types, emit a Named entry so field projection
            // can be resolved in importing modules via lower_import_value_type.
            match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    let item = module.items().get(*item_id)?;
                    let name = item_type_name(item);
                    Some(ImportValueType::Named {
                        type_name: name,
                        arguments: Vec::new(),
                    })
                }
                ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                    let binding = module.imports().get(*import_id)?;
                    let name = binding.imported_name.text().to_owned();
                    Some(ImportValueType::Named {
                        type_name: name,
                        arguments: Vec::new(),
                    })
                }
                _ => None,
            }
        }
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
        TypeKind::RecordTransform { transform, source } => {
            let source = import_value_type(module, *source)?;
            apply_record_row_transform_import_value_type(transform, source)
        }
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

fn item_type_name(item: &Item) -> String {
    match item {
        Item::Type(item) => item.name.text().to_owned(),
        Item::Class(item) => item.name.text().to_owned(),
        Item::Domain(item) => item.name.text().to_owned(),
        Item::SourceProviderContract(item) => {
            item.provider.key().unwrap_or("<provider>").to_owned()
        }
        other => format!("{:?}", other.kind()),
    }
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
        TypeKind::Tuple(_)
        | TypeKind::Record(_)
        | TypeKind::RecordTransform { .. }
        | TypeKind::Arrow { .. } => None,
    }
}

fn apply_record_row_transform_import_value_type(
    transform: &crate::RecordRowTransform,
    source: ImportValueType,
) -> Option<ImportValueType> {
    let ImportValueType::Record(fields) = source else {
        return None;
    };
    let field_index = fields
        .iter()
        .enumerate()
        .map(|(index, field)| (field.name.as_ref(), index))
        .collect::<std::collections::HashMap<_, _>>();
    match transform {
        crate::RecordRowTransform::Pick(labels) => labels
            .iter()
            .map(|label| fields.get(*field_index.get(label.text())?).cloned())
            .collect::<Option<Vec<_>>>()
            .map(ImportValueType::Record),
        crate::RecordRowTransform::Omit(labels) => {
            let omitted = labels
                .iter()
                .map(|label| field_index.get(label.text()).copied())
                .collect::<Option<std::collections::HashSet<_>>>()?;
            Some(ImportValueType::Record(
                fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !omitted.contains(index))
                    .map(|(_, field)| field.clone())
                    .collect(),
            ))
        }
        crate::RecordRowTransform::Optional(labels)
        | crate::RecordRowTransform::Defaulted(labels) => Some(ImportValueType::Record(
            fields
                .iter()
                .map(|field| {
                    if labels
                        .iter()
                        .any(|label| label.text() == field.name.as_ref())
                    {
                        ImportRecordField {
                            name: field.name.clone(),
                            ty: match &field.ty {
                                ImportValueType::Option(_) => field.ty.clone(),
                                other => ImportValueType::Option(Box::new(other.clone())),
                            },
                        }
                    } else {
                        field.clone()
                    }
                })
                .collect(),
        )),
        crate::RecordRowTransform::Required(labels) => Some(ImportValueType::Record(
            fields
                .iter()
                .map(|field| {
                    if labels
                        .iter()
                        .any(|label| label.text() == field.name.as_ref())
                    {
                        ImportRecordField {
                            name: field.name.clone(),
                            ty: match &field.ty {
                                ImportValueType::Option(inner) => inner.as_ref().clone(),
                                other => other.clone(),
                            },
                        }
                    } else {
                        field.clone()
                    }
                })
                .collect(),
        )),
        crate::RecordRowTransform::Rename(renames) => {
            let renamed = renames
                .iter()
                .map(|rename| Some((field_index.get(rename.from.text()).copied()?, rename)))
                .collect::<Option<std::collections::HashMap<_, _>>>()?;
            let mut result = Vec::with_capacity(fields.len());
            let mut seen = std::collections::HashSet::with_capacity(fields.len());
            for (index, field) in fields.iter().enumerate() {
                let name = renamed
                    .get(&index)
                    .map(|rename| rename.to.text().to_owned().into_boxed_str())
                    .unwrap_or_else(|| field.name.clone());
                if !seen.insert(name.clone()) {
                    return None;
                }
                result.push(ImportRecordField {
                    name,
                    ty: field.ty.clone(),
                });
            }
            Some(ImportValueType::Record(result))
        }
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
            | ImportBindingMetadata::IntrinsicValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::Domain { .. }
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::InstanceMember { .. } => None,
        },
        ResolutionState::Resolved(TypeResolution::Item(_))
        | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
        | ResolutionState::Unresolved => None,
    }
}

// ---------------------------------------------------------------------------
// Polymorphic-aware import value type conversion
// ---------------------------------------------------------------------------

type TypeParamMap = HashMap<TypeParameterId, usize>;

/// Convert a HIR type to `ImportValueType`, supporting type parameters and named types.
fn poly_import_value_type(
    module: &Module,
    ty: TypeId,
    params: &TypeParamMap,
) -> Option<ImportValueType> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Name(reference) => poly_name_import_value_type(module, reference, params),
        TypeKind::Tuple(elements) => Some(ImportValueType::Tuple(
            elements
                .iter()
                .map(|element| poly_import_value_type(module, *element, params))
                .collect::<Option<Vec<_>>>()?,
        )),
        TypeKind::Record(fields) => Some(ImportValueType::Record(
            fields
                .iter()
                .map(|field| {
                    Some(ImportRecordField {
                        name: field.label.text().into(),
                        ty: poly_import_value_type(module, field.ty, params)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        TypeKind::RecordTransform { transform, source } => {
            let source = poly_import_value_type(module, *source, params)?;
            apply_record_row_transform_import_value_type(transform, source)
        }
        TypeKind::Arrow { parameter, result } => Some(ImportValueType::Arrow {
            parameter: Box::new(poly_import_value_type(module, *parameter, params)?),
            result: Box::new(poly_import_value_type(module, *result, params)?),
        }),
        TypeKind::Apply { .. } => poly_applied_import_value_type(module, ty, params),
    }
}

/// Handle a bare name reference: builtin primitives, type parameters, or same-module items.
fn poly_name_import_value_type(
    module: &Module,
    reference: &TypeReference,
    params: &TypeParamMap,
) -> Option<ImportValueType> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
            primitive_import_value_type_from_builtin(*builtin)
        }
        ResolutionState::Resolved(TypeResolution::TypeParameter(param_id)) => {
            let &index = params.get(param_id)?;
            let name = module
                .type_parameters()
                .get(*param_id)
                .map(|p| p.name.text().to_owned())
                .unwrap_or_else(|| format!("T{}", index + 1));
            Some(ImportValueType::TypeVariable { index, name })
        }
        ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
            let type_name = item_type_name(&module.items()[*item_id]);
            Some(ImportValueType::Named {
                type_name,
                arguments: Vec::new(),
            })
        }
        ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
            let binding = module.imports().get(*import_id)?;
            let name = binding.imported_name.text().to_owned();
            Some(ImportValueType::Named {
                type_name: name,
                arguments: Vec::new(),
            })
        }
        _ => None,
    }
}

fn primitive_import_value_type_from_builtin(
    builtin: crate::BuiltinType,
) -> Option<ImportValueType> {
    match builtin {
        crate::BuiltinType::Int
        | crate::BuiltinType::Float
        | crate::BuiltinType::Decimal
        | crate::BuiltinType::BigInt
        | crate::BuiltinType::Bool
        | crate::BuiltinType::Text
        | crate::BuiltinType::Unit
        | crate::BuiltinType::Bytes => Some(ImportValueType::Primitive(builtin)),
        _ => None,
    }
}

/// Handle type application: builtin constructors, same-module named types, and named fallback.
fn poly_applied_import_value_type(
    module: &Module,
    ty: TypeId,
    params: &TypeParamMap,
) -> Option<ImportValueType> {
    let (constructor, arguments) = poly_flatten_type_application(module, ty)?;
    // Try builtin constructor first
    match &constructor {
        PolyTypeConstructor::Resolved(ResolvedTypeConstructor::Builtin(builtin)) => {
            let result = match (builtin, arguments.len()) {
                (crate::BuiltinType::List, 1) => Some(ImportValueType::List(Box::new(
                    poly_import_value_type(module, arguments[0], params)?,
                ))),
                (crate::BuiltinType::Map, 2) => Some(ImportValueType::Map {
                    key: Box::new(poly_import_value_type(module, arguments[0], params)?),
                    value: Box::new(poly_import_value_type(module, arguments[1], params)?),
                }),
                (crate::BuiltinType::Set, 1) => Some(ImportValueType::Set(Box::new(
                    poly_import_value_type(module, arguments[0], params)?,
                ))),
                (crate::BuiltinType::Option, 1) => Some(ImportValueType::Option(Box::new(
                    poly_import_value_type(module, arguments[0], params)?,
                ))),
                (crate::BuiltinType::Result, 2) => Some(ImportValueType::Result {
                    error: Box::new(poly_import_value_type(module, arguments[0], params)?),
                    value: Box::new(poly_import_value_type(module, arguments[1], params)?),
                }),
                (crate::BuiltinType::Validation, 2) => Some(ImportValueType::Validation {
                    error: Box::new(poly_import_value_type(module, arguments[0], params)?),
                    value: Box::new(poly_import_value_type(module, arguments[1], params)?),
                }),
                (crate::BuiltinType::Signal, 1) => Some(ImportValueType::Signal(Box::new(
                    poly_import_value_type(module, arguments[0], params)?,
                ))),
                (crate::BuiltinType::Task, 2) => Some(ImportValueType::Task {
                    error: Box::new(poly_import_value_type(module, arguments[0], params)?),
                    value: Box::new(poly_import_value_type(module, arguments[1], params)?),
                }),
                _ => None,
            };
            if result.is_some() {
                return result;
            }
        }
        PolyTypeConstructor::Resolved(ResolvedTypeConstructor::Bundle(
            ImportBundleKind::BuiltinOption,
        )) if arguments.len() == 1 => {
            return Some(ImportValueType::Option(Box::new(
                poly_import_value_type(module, arguments[0], params)?,
            )));
        }
        _ => {}
    }
    // Named type constructor (same-module item or fallback)
    let type_name = match constructor {
        PolyTypeConstructor::Named(name) => name,
        _ => return None,
    };
    let args = arguments
        .iter()
        .map(|arg| poly_import_value_type(module, *arg, params))
        .collect::<Option<Vec<_>>>()?;
    Some(ImportValueType::Named {
        type_name,
        arguments: args,
    })
}

enum PolyTypeConstructor {
    Resolved(ResolvedTypeConstructor),
    Named(String),
}

fn poly_flatten_type_application(
    module: &Module,
    ty: TypeId,
) -> Option<(PolyTypeConstructor, Vec<TypeId>)> {
    let type_node = module.types().get(ty)?;
    match &type_node.kind {
        TypeKind::Apply { callee, arguments } => {
            let (constructor, mut flattened) = poly_flatten_type_application(module, *callee)?;
            flattened.extend(arguments.iter().copied());
            Some((constructor, flattened))
        }
        TypeKind::Name(reference) => {
            if let Some(resolved) = resolve_type_constructor(module, reference) {
                return Some((PolyTypeConstructor::Resolved(resolved), Vec::new()));
            }
            // Same-module item: extract type name
            if let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
                reference.resolution.as_ref()
            {
                let name = item_type_name(&module.items()[*item_id]);
                return Some((PolyTypeConstructor::Named(name), Vec::new()));
            }
            // Imported type: use the imported name
            if let ResolutionState::Resolved(TypeResolution::Import(import_id)) =
                reference.resolution.as_ref()
            {
                if let Some(binding) = module.imports().get(*import_id) {
                    let name = binding.imported_name.text().to_owned();
                    return Some((PolyTypeConstructor::Named(name), Vec::new()));
                }
            }
            None
        }
        TypeKind::Tuple(_)
        | TypeKind::Record(_)
        | TypeKind::RecordTransform { .. }
        | TypeKind::Arrow { .. } => None,
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

/// Extract record fields from a type alias that resolves directly to a record.
/// Returns `Some(fields)` only for zero-parameter aliases of the form
/// `type Foo = { field: Type, ... }`. Parameterised types and sum types return `None`.
fn extract_type_record_fields(
    module: &Module,
    item: &crate::TypeItem,
) -> Option<Vec<ImportRecordField>> {
    // Only monomorphic record aliases carry stable field lists.
    if !item.parameters.is_empty() {
        return None;
    }
    let TypeItemBody::Alias(alias) = &item.body else {
        return None;
    };
    match import_value_type(module, *alias)? {
        ImportValueType::Record(fields) => Some(fields),
        _ => None,
    }
}
