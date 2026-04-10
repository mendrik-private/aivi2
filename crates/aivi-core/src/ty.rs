use std::{fmt, rc::Rc};

use aivi_hir::{
    BuiltinType, GateType as HirGateType, ImportId as HirImportId, ImportTypeDefinition,
    ImportValueType, ItemId as HirItemId, TypeParameterId as HirTypeParameterId,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordField {
    pub name: Box<str>,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Primitive(BuiltinType),
    TypeParameter {
        parameter: HirTypeParameterId,
        name: Box<str>,
    },
    Tuple(Vec<Type>),
    Record(Vec<RecordField>),
    Arrow {
        parameter: Box<Type>,
        result: Box<Type>,
    },
    List(Box<Type>),
    Map {
        key: Box<Type>,
        value: Box<Type>,
    },
    Set(Box<Type>),
    Option(Box<Type>),
    Result {
        error: Box<Type>,
        value: Box<Type>,
    },
    Validation {
        error: Box<Type>,
        value: Box<Type>,
    },
    Signal(Box<Type>),
    Task {
        error: Box<Type>,
        value: Box<Type>,
    },
    Domain {
        item: HirItemId,
        name: Box<str>,
        arguments: Vec<Type>,
    },
    OpaqueItem {
        item: HirItemId,
        name: Box<str>,
        arguments: Vec<Type>,
    },
    OpaqueImport {
        import: HirImportId,
        name: Box<str>,
        arguments: Vec<Type>,
    },
}

impl Type {
    pub fn lower(root: &HirGateType) -> Self {
        #[allow(clippy::enum_variant_names)]
        enum Task<'a> {
            Visit(&'a HirGateType),
            BuildTuple(usize),
            BuildRecord(Vec<Box<str>>),
            BuildArrow,
            BuildList,
            BuildMap,
            BuildSet,
            BuildOption,
            BuildResult,
            BuildValidation,
            BuildSignal,
            BuildTask,
            BuildDomain {
                item: HirItemId,
                name: Box<str>,
                arguments: usize,
            },
            BuildOpaqueItem {
                item: HirItemId,
                name: Box<str>,
                arguments: usize,
            },
            BuildOpaqueImport {
                import: HirImportId,
                name: Box<str>,
                arguments: usize,
            },
        }

        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(ty) => match ty {
                    HirGateType::Primitive(builtin) => values.push(Self::Primitive(*builtin)),
                    HirGateType::TypeParameter { parameter, name } => {
                        values.push(Self::TypeParameter {
                            parameter: *parameter,
                            name: name.clone().into_boxed_str(),
                        });
                    }
                    HirGateType::Tuple(elements) => {
                        tasks.push(Task::BuildTuple(elements.len()));
                        for element in elements.iter().rev() {
                            tasks.push(Task::Visit(element));
                        }
                    }
                    HirGateType::Record(fields) => {
                        tasks.push(Task::BuildRecord(
                            fields
                                .iter()
                                .map(|field| field.name.clone().into_boxed_str())
                                .collect(),
                        ));
                        for field in fields.iter().rev() {
                            tasks.push(Task::Visit(&field.ty));
                        }
                    }
                    HirGateType::Arrow { parameter, result } => {
                        tasks.push(Task::BuildArrow);
                        tasks.push(Task::Visit(result));
                        tasks.push(Task::Visit(parameter));
                    }
                    HirGateType::List(element) => {
                        tasks.push(Task::BuildList);
                        tasks.push(Task::Visit(element));
                    }
                    HirGateType::Map { key, value } => {
                        tasks.push(Task::BuildMap);
                        tasks.push(Task::Visit(value));
                        tasks.push(Task::Visit(key));
                    }
                    HirGateType::Set(element) => {
                        tasks.push(Task::BuildSet);
                        tasks.push(Task::Visit(element));
                    }
                    HirGateType::Option(element) => {
                        tasks.push(Task::BuildOption);
                        tasks.push(Task::Visit(element));
                    }
                    HirGateType::Result { error, value } => {
                        tasks.push(Task::BuildResult);
                        tasks.push(Task::Visit(value));
                        tasks.push(Task::Visit(error));
                    }
                    HirGateType::Validation { error, value } => {
                        tasks.push(Task::BuildValidation);
                        tasks.push(Task::Visit(value));
                        tasks.push(Task::Visit(error));
                    }
                    HirGateType::Signal(inner) => {
                        tasks.push(Task::BuildSignal);
                        tasks.push(Task::Visit(inner));
                    }
                    HirGateType::Task { error, value } => {
                        tasks.push(Task::BuildTask);
                        tasks.push(Task::Visit(value));
                        tasks.push(Task::Visit(error));
                    }
                    HirGateType::Domain {
                        item,
                        name,
                        arguments,
                    } => {
                        tasks.push(Task::BuildDomain {
                            item: *item,
                            name: name.clone().into_boxed_str(),
                            arguments: arguments.len(),
                        });
                        for argument in arguments.iter().rev() {
                            tasks.push(Task::Visit(argument));
                        }
                    }
                    HirGateType::OpaqueItem {
                        item,
                        name,
                        arguments,
                    } => {
                        tasks.push(Task::BuildOpaqueItem {
                            item: *item,
                            name: name.clone().into_boxed_str(),
                            arguments: arguments.len(),
                        });
                        for argument in arguments.iter().rev() {
                            tasks.push(Task::Visit(argument));
                        }
                    }
                    HirGateType::OpaqueImport {
                        import,
                        name,
                        arguments,
                        definition,
                    } => match definition.as_deref() {
                        Some(ImportTypeDefinition::Alias(alias)) => {
                            let substitutions = Rc::from(
                                arguments
                                    .iter()
                                    .map(Self::lower)
                                    .collect::<Vec<_>>()
                                    .into_boxed_slice(),
                            );
                            values
                                .push(Self::lower_import_with_substitutions(alias, substitutions));
                        }
                        Some(ImportTypeDefinition::Sum(_)) | None => {
                            tasks.push(Task::BuildOpaqueImport {
                                import: *import,
                                name: name.clone().into_boxed_str(),
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument));
                            }
                        }
                    },
                },
                Task::BuildTuple(len) => {
                    let tuple = Self::Tuple(drain_tail(&mut values, len));
                    values.push(tuple);
                }
                Task::BuildRecord(names) => {
                    let len = names.len();
                    let fields = names
                        .into_iter()
                        .zip(drain_tail(&mut values, len))
                        .map(|(name, ty)| RecordField { name, ty })
                        .collect();
                    values.push(Self::Record(fields));
                }
                Task::BuildArrow => {
                    let mut drained = drain_tail(&mut values, 2);
                    let parameter = drained.remove(0);
                    let result = drained.remove(0);
                    values.push(Self::Arrow {
                        parameter: Box::new(parameter),
                        result: Box::new(result),
                    });
                }
                Task::BuildList => {
                    let child = values.pop().expect("list child should exist");
                    values.push(Self::List(Box::new(child)));
                }
                Task::BuildMap => {
                    let mut drained = drain_tail(&mut values, 2);
                    let key = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Map {
                        key: Box::new(key),
                        value: Box::new(value),
                    });
                }
                Task::BuildSet => {
                    let child = values.pop().expect("set child should exist");
                    values.push(Self::Set(Box::new(child)));
                }
                Task::BuildOption => {
                    let child = values.pop().expect("option child should exist");
                    values.push(Self::Option(Box::new(child)));
                }
                Task::BuildResult => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Result {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildValidation => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Validation {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildSignal => {
                    let child = values.pop().expect("signal child should exist");
                    values.push(Self::Signal(Box::new(child)));
                }
                Task::BuildTask => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Task {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildDomain {
                    item,
                    name,
                    arguments,
                } => {
                    let arguments = drain_tail(&mut values, arguments);
                    values.push(Self::Domain {
                        item,
                        name,
                        arguments,
                    });
                }
                Task::BuildOpaqueItem {
                    item,
                    name,
                    arguments,
                } => {
                    let arguments = drain_tail(&mut values, arguments);
                    values.push(Self::OpaqueItem {
                        item,
                        name,
                        arguments,
                    });
                }
                Task::BuildOpaqueImport {
                    import,
                    name,
                    arguments,
                } => {
                    let arguments = drain_tail(&mut values, arguments);
                    values.push(Self::OpaqueImport {
                        import,
                        name,
                        arguments,
                    });
                }
            }
        }

        values
            .pop()
            .expect("typed-core type lowering should always produce one result")
    }

    pub fn lower_import(root: &ImportValueType) -> Self {
        Self::lower_import_with_substitutions(root, Rc::from(Vec::<Type>::new().into_boxed_slice()))
    }

    fn lower_import_with_substitutions(root: &ImportValueType, substitutions: Rc<[Type]>) -> Self {
        #[allow(clippy::enum_variant_names)]
        enum Task<'a> {
            Visit(&'a ImportValueType, Rc<[Type]>),
            BuildTuple(usize),
            BuildRecord(Vec<Box<str>>),
            BuildArrow,
            BuildList,
            BuildMap,
            BuildSet,
            BuildOption,
            BuildResult,
            BuildValidation,
            BuildSignal,
            BuildTask,
            BuildOpaqueImport {
                name: Box<str>,
                arguments: usize,
            },
            EnterAlias {
                alias: &'a ImportValueType,
                arguments: usize,
            },
        }

        let mut tasks = vec![Task::Visit(root, substitutions)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(ty, substitutions) => match ty {
                    ImportValueType::Primitive(builtin) => values.push(Self::Primitive(*builtin)),
                    ImportValueType::Tuple(elements) => {
                        tasks.push(Task::BuildTuple(elements.len()));
                        for element in elements.iter().rev() {
                            tasks.push(Task::Visit(element, substitutions.clone()));
                        }
                    }
                    ImportValueType::Record(fields) => {
                        tasks.push(Task::BuildRecord(
                            fields.iter().map(|field| field.name.clone()).collect(),
                        ));
                        for field in fields.iter().rev() {
                            tasks.push(Task::Visit(&field.ty, substitutions.clone()));
                        }
                    }
                    ImportValueType::Arrow { parameter, result } => {
                        tasks.push(Task::BuildArrow);
                        tasks.push(Task::Visit(result, substitutions.clone()));
                        tasks.push(Task::Visit(parameter, substitutions.clone()));
                    }
                    ImportValueType::List(element) => {
                        tasks.push(Task::BuildList);
                        tasks.push(Task::Visit(element, substitutions.clone()));
                    }
                    ImportValueType::Map { key, value } => {
                        tasks.push(Task::BuildMap);
                        tasks.push(Task::Visit(value, substitutions.clone()));
                        tasks.push(Task::Visit(key, substitutions.clone()));
                    }
                    ImportValueType::Set(element) => {
                        tasks.push(Task::BuildSet);
                        tasks.push(Task::Visit(element, substitutions.clone()));
                    }
                    ImportValueType::Option(element) => {
                        tasks.push(Task::BuildOption);
                        tasks.push(Task::Visit(element, substitutions.clone()));
                    }
                    ImportValueType::Result { error, value } => {
                        tasks.push(Task::BuildResult);
                        tasks.push(Task::Visit(value, substitutions.clone()));
                        tasks.push(Task::Visit(error, substitutions.clone()));
                    }
                    ImportValueType::Validation { error, value } => {
                        tasks.push(Task::BuildValidation);
                        tasks.push(Task::Visit(value, substitutions.clone()));
                        tasks.push(Task::Visit(error, substitutions.clone()));
                    }
                    ImportValueType::Signal(inner) => {
                        tasks.push(Task::BuildSignal);
                        tasks.push(Task::Visit(inner, substitutions.clone()));
                    }
                    ImportValueType::Task { error, value } => {
                        tasks.push(Task::BuildTask);
                        tasks.push(Task::Visit(value, substitutions.clone()));
                        tasks.push(Task::Visit(error, substitutions.clone()));
                    }
                    ImportValueType::TypeVariable { index, .. } => {
                        if let Some(ty) = substitutions.get(*index).cloned() {
                            values.push(ty);
                        } else {
                            values.push(Self::OpaqueImport {
                                import: aivi_hir::ImportId::from_raw(u32::MAX),
                                name: "".into(),
                                arguments: Vec::new(),
                            });
                        }
                    }
                    ImportValueType::Named {
                        type_name,
                        arguments,
                        definition,
                    } => match definition.as_deref() {
                        Some(ImportTypeDefinition::Alias(alias)) => {
                            tasks.push(Task::EnterAlias {
                                alias,
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument, substitutions.clone()));
                            }
                        }
                        Some(ImportTypeDefinition::Sum(_)) | None => {
                            tasks.push(Task::BuildOpaqueImport {
                                name: type_name.clone().into_boxed_str(),
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument, substitutions.clone()));
                            }
                        }
                    },
                },
                Task::BuildTuple(len) => {
                    let tuple = Self::Tuple(drain_tail(&mut values, len));
                    values.push(tuple);
                }
                Task::BuildRecord(names) => {
                    let len = names.len();
                    let record = Self::Record(
                        names
                            .into_iter()
                            .zip(drain_tail(&mut values, len))
                            .map(|(name, ty)| RecordField { name, ty })
                            .collect(),
                    );
                    values.push(record);
                }
                Task::BuildArrow => {
                    let mut drained = drain_tail(&mut values, 2);
                    let parameter = drained.remove(0);
                    let result = drained.remove(0);
                    values.push(Self::Arrow {
                        parameter: Box::new(parameter),
                        result: Box::new(result),
                    });
                }
                Task::BuildList => {
                    let child = values.pop().expect("list child should exist");
                    values.push(Self::List(Box::new(child)));
                }
                Task::BuildMap => {
                    let mut drained = drain_tail(&mut values, 2);
                    let key = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Map {
                        key: Box::new(key),
                        value: Box::new(value),
                    });
                }
                Task::BuildSet => {
                    let child = values.pop().expect("set child should exist");
                    values.push(Self::Set(Box::new(child)));
                }
                Task::BuildOption => {
                    let child = values.pop().expect("option child should exist");
                    values.push(Self::Option(Box::new(child)));
                }
                Task::BuildResult => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Result {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildValidation => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Validation {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildSignal => {
                    let child = values.pop().expect("signal child should exist");
                    values.push(Self::Signal(Box::new(child)));
                }
                Task::BuildTask => {
                    let mut drained = drain_tail(&mut values, 2);
                    let error = drained.remove(0);
                    let value = drained.remove(0);
                    values.push(Self::Task {
                        error: Box::new(error),
                        value: Box::new(value),
                    });
                }
                Task::BuildOpaqueImport { name, arguments } => {
                    let arguments = drain_tail(&mut values, arguments);
                    values.push(Self::OpaqueImport {
                        import: aivi_hir::ImportId::from_raw(u32::MAX),
                        name,
                        arguments,
                    });
                }
                Task::EnterAlias { alias, arguments } => {
                    let substitutions =
                        Rc::from(drain_tail(&mut values, arguments).into_boxed_slice());
                    tasks.push(Task::Visit(alias, substitutions));
                }
            }
        }

        values
            .pop()
            .expect("typed-core import type lowering should always produce one result")
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Self::Primitive(BuiltinType::Bool))
    }

    pub fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub fn same_shape(&self, other: &Self) -> bool {
        self == other
    }
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("requested more lowered values than available");
    values.drain(split..).collect()
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            Type::TypeParameter { name, .. } => write!(f, "{name}"),
            Type::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            Type::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            Type::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            Type::List(element) => write!(f, "List {element}"),
            Type::Map { key, value } => write!(f, "Map {key} {value}"),
            Type::Set(element) => write!(f, "Set {element}"),
            Type::Option(element) => write!(f, "Option {element}"),
            Type::Result { error, value } => write!(f, "Result {error} {value}"),
            Type::Validation { error, value } => write!(f, "Validation {error} {value}"),
            Type::Signal(inner) => write!(f, "Signal {inner}"),
            Type::Task { error, value } => write!(f, "Task {error} {value}"),
            Type::Domain {
                name, arguments, ..
            }
            | Type::OpaqueItem {
                name, arguments, ..
            }
            | Type::OpaqueImport {
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

fn builtin_type_name(builtin: BuiltinType) -> &'static str {
    match builtin {
        BuiltinType::Int => "Int",
        BuiltinType::Float => "Float",
        BuiltinType::Decimal => "Decimal",
        BuiltinType::BigInt => "BigInt",
        BuiltinType::Bool => "Bool",
        BuiltinType::Text => "Text",
        BuiltinType::Unit => "Unit",
        BuiltinType::Bytes => "Bytes",
        BuiltinType::List => "List",
        BuiltinType::Map => "Map",
        BuiltinType::Set => "Set",
        BuiltinType::Option => "Option",
        BuiltinType::Result => "Result",
        BuiltinType::Validation => "Validation",
        BuiltinType::Signal => "Signal",
        BuiltinType::Task => "Task",
    }
}

#[cfg(test)]
mod tests {
    use super::Type;
    use aivi_hir::{
        BuiltinType, GateType as HirGateType, ImportId, ImportTypeDefinition, ImportValueType,
    };

    #[test]
    fn lower_hir_import_alias_substitutes_type_arguments() {
        let ty = HirGateType::OpaqueImport {
            import: ImportId::from_raw(0),
            name: "Envelope".into(),
            arguments: vec![HirGateType::Option(Box::new(HirGateType::Primitive(
                BuiltinType::Int,
            )))],
            definition: Some(Box::new(ImportTypeDefinition::Alias(
                ImportValueType::TypeVariable {
                    index: 0,
                    name: "A".into(),
                },
            ))),
        };

        assert_eq!(
            Type::lower(&ty),
            Type::Option(Box::new(Type::Primitive(BuiltinType::Int)))
        );
    }

    #[test]
    fn lower_import_alias_substitutes_direct_arguments() {
        let ty = ImportValueType::Named {
            type_name: "Envelope".into(),
            arguments: vec![ImportValueType::Primitive(BuiltinType::Int)],
            definition: Some(Box::new(ImportTypeDefinition::Alias(
                ImportValueType::TypeVariable {
                    index: 0,
                    name: "A".into(),
                },
            ))),
        };

        assert_eq!(Type::lower_import(&ty), Type::Primitive(BuiltinType::Int));
    }

    #[test]
    fn lower_import_alias_substitutes_outer_arguments_inside_nested_aliases() {
        let ty = ImportValueType::Named {
            type_name: "Wrap".into(),
            arguments: vec![ImportValueType::Primitive(BuiltinType::Int)],
            definition: Some(Box::new(ImportTypeDefinition::Alias(
                ImportValueType::Named {
                    type_name: "Envelope".into(),
                    arguments: vec![ImportValueType::TypeVariable {
                        index: 0,
                        name: "A".into(),
                    }],
                    definition: Some(Box::new(ImportTypeDefinition::Alias(
                        ImportValueType::Option(Box::new(ImportValueType::TypeVariable {
                            index: 0,
                            name: "B".into(),
                        })),
                    ))),
                },
            ))),
        };

        assert_eq!(
            Type::lower_import(&ty),
            Type::Option(Box::new(Type::Primitive(BuiltinType::Int)))
        );
    }
}
