use std::collections::HashMap;

use aivi_typing::{PrimitiveType, SourceContractType, SourceTypeAtom, SourceTypeParameter};

use crate::{BuiltinType, Item, ItemId, Module};

/// One source contract type mapped onto the current resolved-HIR program type surface.
///
/// This stays intentionally narrower than full HIR type expressions: it records only the builtins,
/// same-module type/domain items, and contract-local placeholders that the current compiler wave
/// can resolve honestly before ordinary option-expression typing exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedSourceContractType {
    Builtin(BuiltinType),
    Item(ItemId),
    ContractParameter(SourceTypeParameter),
    Apply {
        callee: ResolvedSourceTypeConstructor,
        arguments: Vec<ResolvedSourceContractType>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedSourceTypeConstructor {
    Builtin(BuiltinType),
    Item(ItemId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceContractResolutionError {
    kind: SourceContractResolutionErrorKind,
}

impl SourceContractResolutionError {
    pub fn kind(&self) -> &SourceContractResolutionErrorKind {
        &self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceContractResolutionErrorKind {
    MissingType {
        name: String,
    },
    AmbiguousType {
        name: String,
    },
    ArityMismatch {
        name: String,
        expected: usize,
        actual: usize,
        item: ItemId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalTypeLookup {
    Missing,
    Ambiguous,
    Unique { item: ItemId, arity: usize },
}

pub struct SourceContractTypeResolver<'a> {
    module: &'a Module,
    local_types: HashMap<String, LocalTypeLookup>,
}

impl<'a> SourceContractTypeResolver<'a> {
    pub fn new(module: &'a Module) -> Self {
        Self {
            module,
            local_types: HashMap::new(),
        }
    }

    pub fn resolve(
        &mut self,
        ty: SourceContractType,
    ) -> Result<ResolvedSourceContractType, SourceContractResolutionError> {
        match ty {
            SourceContractType::Atom(atom) => self.resolve_atom(atom),
            SourceContractType::List(element) => Ok(ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Builtin(BuiltinType::List),
                arguments: vec![self.resolve_atom(element)?],
            }),
            SourceContractType::Map { key, value } => Ok(ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Item(self.resolve_named_type("Map", 2)?),
                arguments: vec![self.resolve_atom(key)?, self.resolve_atom(value)?],
            }),
            SourceContractType::Signal(payload) => Ok(ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal),
                arguments: vec![self.resolve(*payload)?],
            }),
        }
    }

    fn resolve_atom(
        &mut self,
        atom: SourceTypeAtom,
    ) -> Result<ResolvedSourceContractType, SourceContractResolutionError> {
        match atom {
            SourceTypeAtom::Primitive(primitive) => Ok(ResolvedSourceContractType::Builtin(
                builtin_primitive(primitive),
            )),
            SourceTypeAtom::Nominal(nominal) => Ok(ResolvedSourceContractType::Item(
                self.resolve_named_type(nominal.as_str(), 0)?,
            )),
            SourceTypeAtom::Parameter(parameter) => {
                Ok(ResolvedSourceContractType::ContractParameter(parameter))
            }
        }
    }

    fn resolve_named_type(
        &mut self,
        name: &str,
        expected_arity: usize,
    ) -> Result<ItemId, SourceContractResolutionError> {
        match self.lookup_local_type(name) {
            LocalTypeLookup::Missing => Err(SourceContractResolutionError {
                kind: SourceContractResolutionErrorKind::MissingType {
                    name: name.to_owned(),
                },
            }),
            LocalTypeLookup::Ambiguous => Err(SourceContractResolutionError {
                kind: SourceContractResolutionErrorKind::AmbiguousType {
                    name: name.to_owned(),
                },
            }),
            LocalTypeLookup::Unique { item, arity } if arity == expected_arity => Ok(item),
            LocalTypeLookup::Unique { item, arity } => Err(SourceContractResolutionError {
                kind: SourceContractResolutionErrorKind::ArityMismatch {
                    name: name.to_owned(),
                    expected: expected_arity,
                    actual: arity,
                    item,
                },
            }),
        }
    }

    fn lookup_local_type(&mut self, name: &str) -> LocalTypeLookup {
        if let Some(lookup) = self.local_types.get(name) {
            return *lookup;
        }

        let mut matches = self
            .module
            .items()
            .iter()
            .filter_map(|(item_id, item)| match item {
                Item::Type(item) => {
                    (item.name.text() == name).then_some((item_id, item.parameters.len()))
                }
                Item::Domain(item) => {
                    (item.name.text() == name).then_some((item_id, item.parameters.len()))
                }
                Item::Value(_)
                | Item::Function(_)
                | Item::Signal(_)
                | Item::Class(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => None,
            });

        let lookup = match (matches.next(), matches.next()) {
            (None, _) => LocalTypeLookup::Missing,
            (Some((item, arity)), None) => LocalTypeLookup::Unique { item, arity },
            (Some(_), Some(_)) => LocalTypeLookup::Ambiguous,
        };
        self.local_types.insert(name.to_owned(), lookup);
        lookup
    }
}

fn builtin_primitive(primitive: PrimitiveType) -> BuiltinType {
    match primitive {
        PrimitiveType::Int => BuiltinType::Int,
        PrimitiveType::Float => BuiltinType::Float,
        PrimitiveType::Decimal => BuiltinType::Decimal,
        PrimitiveType::BigInt => BuiltinType::BigInt,
        PrimitiveType::Bool => BuiltinType::Bool,
        PrimitiveType::Text => BuiltinType::Text,
        PrimitiveType::Unit => BuiltinType::Unit,
        PrimitiveType::Bytes => BuiltinType::Bytes,
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::{ByteIndex, FileId, SourceSpan, Span};
    use aivi_typing::{SourceNominalType, SourceTypeAtom, SourceTypeParameter};

    use crate::{
        Item, ItemHeader, Module, Name, NonEmpty, TypeItem, TypeItemBody, TypeNode, TypeParameter,
        TypeReference, TypeResolution, TypeVariant,
    };

    use super::*;

    fn span() -> SourceSpan {
        SourceSpan::new(
            FileId::new(0),
            Span::new(ByteIndex::new(0), ByteIndex::new(1)),
        )
    }

    fn name(text: &str) -> Name {
        Name::new(text, span()).expect("test name should stay valid")
    }

    fn builtin_type(module: &mut Module, builtin: BuiltinType) -> crate::TypeId {
        let path = crate::NamePath::from_vec(vec![name(builtin_name(builtin))])
            .expect("builtin path should stay valid");
        module
            .alloc_type(TypeNode {
                span: span(),
                kind: crate::TypeKind::Name(TypeReference::resolved(
                    path,
                    TypeResolution::Builtin(builtin),
                )),
            })
            .expect("builtin type allocation should fit")
    }

    fn push_sum_type(module: &mut Module, item_name: &str, parameters: &[&str]) -> ItemId {
        let parameters = parameters
            .iter()
            .map(|parameter| {
                module
                    .alloc_type_parameter(TypeParameter {
                        span: span(),
                        name: name(parameter),
                    })
                    .expect("type parameter allocation should fit")
            })
            .collect::<Vec<_>>();
        module
            .push_item(Item::Type(TypeItem {
                header: ItemHeader {
                    span: span(),
                    decorators: Vec::new(),
                },
                name: name(item_name),
                parameters,
                body: TypeItemBody::Sum(NonEmpty::new(
                    TypeVariant {
                        span: span(),
                        name: name("Only"),
                        fields: Vec::new(),
                    },
                    Vec::new(),
                )),
            }))
            .expect("type item allocation should fit")
    }

    fn push_domain(module: &mut Module, item_name: &str) -> ItemId {
        let carrier = builtin_type(module, BuiltinType::Int);
        module
            .push_item(Item::Domain(crate::DomainItem {
                header: ItemHeader {
                    span: span(),
                    decorators: Vec::new(),
                },
                name: name(item_name),
                parameters: Vec::new(),
                carrier,
                members: Vec::new(),
            }))
            .expect("domain item allocation should fit")
    }

    fn builtin_name(builtin: BuiltinType) -> &'static str {
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

    #[test]
    fn resolves_source_contract_types_into_builtins_and_same_module_items() {
        let mut module = Module::new(FileId::new(0));
        let duration = push_domain(&mut module, "Duration");
        let retry = push_domain(&mut module, "Retry");
        let decode_mode = push_sum_type(&mut module, "DecodeMode", &[]);
        let stream_mode = push_sum_type(&mut module, "StreamMode", &[]);
        let fs_watch_event = push_sum_type(&mut module, "FsWatchEvent", &[]);
        let map = push_sum_type(&mut module, "Map", &["K", "V"]);

        let mut resolver = SourceContractTypeResolver::new(&module);

        assert_eq!(
            resolver
                .resolve(SourceContractType::nominal(SourceNominalType::Duration))
                .expect("Duration should resolve"),
            ResolvedSourceContractType::Item(duration)
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::nominal(SourceNominalType::Retry))
                .expect("Retry should resolve"),
            ResolvedSourceContractType::Item(retry)
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::nominal(SourceNominalType::DecodeMode))
                .expect("DecodeMode should resolve"),
            ResolvedSourceContractType::Item(decode_mode)
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::nominal(SourceNominalType::StreamMode))
                .expect("StreamMode should resolve"),
            ResolvedSourceContractType::Item(stream_mode)
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::list(SourceTypeAtom::nominal(
                    SourceNominalType::FsWatchEvent,
                )))
                .expect("List FsWatchEvent should resolve"),
            ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Builtin(BuiltinType::List),
                arguments: vec![ResolvedSourceContractType::Item(fs_watch_event)],
            }
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::map(
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                    SourceTypeAtom::primitive(PrimitiveType::Text),
                ))
                .expect("Map Text Text should resolve"),
            ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Item(map),
                arguments: vec![
                    ResolvedSourceContractType::Builtin(BuiltinType::Text),
                    ResolvedSourceContractType::Builtin(BuiltinType::Text),
                ],
            }
        );
        assert_eq!(
            resolver
                .resolve(SourceContractType::signal(SourceTypeAtom::parameter(
                    SourceTypeParameter::B,
                )))
                .expect("Signal B should resolve"),
            ResolvedSourceContractType::Apply {
                callee: ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal),
                arguments: vec![ResolvedSourceContractType::ContractParameter(
                    SourceTypeParameter::B,
                )],
            }
        );
    }

    #[test]
    fn rejects_missing_nominal_types_and_wrong_constructor_arities() {
        let mut module = Module::new(FileId::new(0));
        let map = push_sum_type(&mut module, "Map", &["K"]);
        let mut resolver = SourceContractTypeResolver::new(&module);

        let missing = resolver
            .resolve(SourceContractType::nominal(SourceNominalType::Retry))
            .expect_err("Retry should be missing");
        assert_eq!(
            missing.kind(),
            &SourceContractResolutionErrorKind::MissingType {
                name: "Retry".to_owned(),
            }
        );

        let wrong_arity = resolver
            .resolve(SourceContractType::map(
                SourceTypeAtom::primitive(PrimitiveType::Text),
                SourceTypeAtom::primitive(PrimitiveType::Text),
            ))
            .expect_err("Map should reject the wrong arity");
        assert_eq!(
            wrong_arity.kind(),
            &SourceContractResolutionErrorKind::ArityMismatch {
                name: "Map".to_owned(),
                expected: 2,
                actual: 1,
                item: map,
            }
        );
    }
}
