use std::collections::HashMap;

use aivi_base::SourceSpan;
use aivi_typing::{
    Closedness, DecodeMode, DecodePlanner, DecodePlanningError, DecodeSchema, ExternalTypeId,
    PrimitiveType, RecordField, ShapeErrorKind, SumVariant, TypeId as StructuralTypeId,
    TypeParameterId as StructuralTypeParameterId, TypeStore,
};

use crate::{
    BuiltinType, DecoratorId, DecoratorPayload, ExprId, ExprKind, Item, ItemId, Module,
    ResolutionState, SignalItem, SourceDecorator, TermReference, TermResolution,
    TypeId as HirTypeId, TypeItemBody, TypeKind, TypeParameterId as HirTypeParameterId,
    TypeResolution, TypeVariant,
};

/// Focused pre-runtime decode-schema handoff for `@source` signals.
///
/// This keeps RFC §14.2 planning explicitly above resolved HIR and below any runtime decoder
/// generation: every source-backed signal may now contribute one typed schema for its published
/// payload, together with the chosen `Strict` / `Permissive` mode and explicit blockers when the
/// current frontend still lacks enough structural evidence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceDecodeElaborationReport {
    nodes: Vec<SourceDecodeNodeElaboration>,
}

impl SourceDecodeElaborationReport {
    pub fn new(nodes: Vec<SourceDecodeNodeElaboration>) -> Self {
        Self { nodes }
    }

    pub fn nodes(&self) -> &[SourceDecodeNodeElaboration] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<SourceDecodeNodeElaboration> {
        self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecodeNodeElaboration {
    pub owner: ItemId,
    pub source_span: SourceSpan,
    pub outcome: SourceDecodeNodeOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeNodeOutcome {
    Planned(SourceDecodePlan),
    Blocked(BlockedSourceDecodeNode),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecodePlan {
    pub mode: DecodeMode,
    pub payload_annotation: HirTypeId,
    pub schema: DecodeSchema,
    pub structural_types: TypeStore,
    pub domain_bindings: Vec<SourceDecodeDomainBinding>,
}

impl SourceDecodePlan {
    pub fn domain_binding(&self, ty: StructuralTypeId) -> Option<&SourceDecodeDomainBinding> {
        self.domain_bindings.iter().find(|binding| binding.ty == ty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecodeDomainBinding {
    pub ty: StructuralTypeId,
    pub domain_item: ItemId,
    pub arguments: Vec<StructuralTypeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedSourceDecodeNode {
    pub mode: Option<DecodeMode>,
    pub payload_annotation: Option<HirTypeId>,
    pub blockers: Vec<SourceDecodeElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeElaborationBlocker {
    MissingSignalAnnotation,
    AnnotationNotSignal {
        span: SourceSpan,
    },
    UnknownDecodeMode {
        span: SourceSpan,
    },
    UnsupportedPayloadType {
        span: SourceSpan,
        kind: SourceDecodeUnsupportedTypeKind,
    },
    InvalidPayloadShape {
        span: SourceSpan,
        kind: ShapeErrorKind,
    },
    DecodePlanning(DecodePlanningError),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceDecodeUnsupportedTypeKind {
    Arrow,
    NestedSignal,
    Task,
}

pub fn elaborate_source_decodes(module: &Module) -> SourceDecodeElaborationReport {
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();

    for (owner, item) in items {
        let Item::Signal(signal) = item else {
            continue;
        };
        let Some((_, source_span, source)) = signal_source_decorator(module, &signal) else {
            continue;
        };
        nodes.push(SourceDecodeNodeElaboration {
            owner,
            source_span,
            outcome: elaborate_source_decode_signal(module, &signal, source),
        });
    }

    SourceDecodeElaborationReport::new(nodes)
}

fn elaborate_source_decode_signal(
    module: &Module,
    signal: &SignalItem,
    source: &SourceDecorator,
) -> SourceDecodeNodeOutcome {
    let mut blockers = Vec::new();
    let mode = match decode_mode_for_source(module, source) {
        Ok(mode) => Some(mode),
        Err(span) => {
            blockers.push(SourceDecodeElaborationBlocker::UnknownDecodeMode { span });
            None
        }
    };

    let Some(annotation) = signal.annotation else {
        blockers.push(SourceDecodeElaborationBlocker::MissingSignalAnnotation);
        return SourceDecodeNodeOutcome::Blocked(BlockedSourceDecodeNode {
            mode,
            payload_annotation: None,
            blockers,
        });
    };

    let lowered = match DecodeTypeLowerer::new(module).lower_source_signal_payload(annotation) {
        Ok(lowered) => Some(lowered),
        Err(error) => {
            blockers.push(error);
            None
        }
    };

    let payload_annotation = lowered.as_ref().map(|lowered| lowered.payload_annotation);

    if let (Some(mode), Some(lowered)) = (mode, lowered) {
        match DecodePlanner::plan(&lowered.structural_types, lowered.subject, mode) {
            Ok(schema) if blockers.is_empty() => {
                return SourceDecodeNodeOutcome::Planned(SourceDecodePlan {
                    mode,
                    payload_annotation: lowered.payload_annotation,
                    schema,
                    structural_types: lowered.structural_types,
                    domain_bindings: lowered.domain_bindings,
                });
            }
            Ok(_) => {}
            Err(error) => blockers.push(SourceDecodeElaborationBlocker::DecodePlanning(error)),
        }
    }

    SourceDecodeNodeOutcome::Blocked(BlockedSourceDecodeNode {
        mode,
        payload_annotation,
        blockers,
    })
}

fn decode_mode_for_source(
    module: &Module,
    source: &SourceDecorator,
) -> Result<DecodeMode, SourceSpan> {
    let Some(options) = source.options else {
        return Ok(DecodeMode::Strict);
    };
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return Ok(DecodeMode::Strict);
    };
    let Some(field) = record
        .fields
        .iter()
        .find(|field| field.label.text() == "decode")
    else {
        return Ok(DecodeMode::Strict);
    };
    resolve_decode_mode_expr(module, field.value, &mut Vec::new())
        .ok_or(module.exprs()[field.value].span)
}

fn resolve_decode_mode_expr(
    module: &Module,
    expr_id: ExprId,
    value_stack: &mut Vec<ItemId>,
) -> Option<DecodeMode> {
    let expr = module.exprs()[expr_id].clone();
    match expr.kind {
        ExprKind::Name(reference) => resolve_decode_mode_name(module, &reference, value_stack),
        _ => None,
    }
}

fn resolve_decode_mode_name(
    module: &Module,
    reference: &TermReference,
    value_stack: &mut Vec<ItemId>,
) -> Option<DecodeMode> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            let item = module.items()[*item_id].clone();
            match item {
                Item::Value(item) => {
                    if value_stack.contains(item_id) {
                        return None;
                    }
                    value_stack.push(*item_id);
                    let mode = resolve_decode_mode_expr(module, item.body, value_stack);
                    let popped = value_stack.pop();
                    debug_assert_eq!(popped, Some(*item_id));
                    mode
                }
                Item::Type(item) => {
                    if item.name.text() != "DecodeMode" {
                        return None;
                    }
                    let TypeItemBody::Sum(variants) = item.body else {
                        return None;
                    };
                    let constructor_name = reference.path.segments().last().text();
                    let variant = variants
                        .iter()
                        .find(|variant| variant.name.text() == constructor_name)?;
                    if !variant.fields.is_empty() {
                        return None;
                    }
                    match constructor_name {
                        "Strict" => Some(DecodeMode::Strict),
                        "Permissive" => Some(DecodeMode::Permissive),
                        _ => None,
                    }
                }
                Item::Function(_)
                | Item::Signal(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => None,
            }
        }
        ResolutionState::Resolved(TermResolution::Local(_))
        | ResolutionState::Resolved(TermResolution::DomainMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
        | ResolutionState::Resolved(TermResolution::Import(_))
        | ResolutionState::Resolved(TermResolution::Builtin(_))
        | ResolutionState::Unresolved => None,
    }
}

fn signal_source_decorator<'a>(
    module: &'a Module,
    item: &'a SignalItem,
) -> Option<(DecoratorId, SourceSpan, &'a SourceDecorator)> {
    item.header.decorators.iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some((*decorator_id, decorator.span, source)),
            _ => None,
        }
    })
}

struct LoweredDecodeType {
    payload_annotation: HirTypeId,
    structural_types: TypeStore,
    subject: StructuralTypeId,
    domain_bindings: Vec<SourceDecodeDomainBinding>,
}

pub(crate) struct DecodeTypeLowerer<'a> {
    module: &'a Module,
    types: TypeStore,
    parameters: HashMap<HirTypeParameterId, StructuralTypeParameterId>,
    externals: HashMap<String, ExternalTypeId>,
    domain_bindings: Vec<SourceDecodeDomainBinding>,
}

impl<'a> DecodeTypeLowerer<'a> {
    fn new(module: &'a Module) -> Self {
        Self::with_type_store(module, TypeStore::new())
    }

    pub(crate) fn with_type_store(module: &'a Module, types: TypeStore) -> Self {
        Self {
            module,
            types,
            parameters: HashMap::new(),
            externals: HashMap::new(),
            domain_bindings: Vec::new(),
        }
    }

    pub(crate) fn types(&self) -> &TypeStore {
        &self.types
    }

    pub(crate) fn lower_type_with_substitutions(
        &mut self,
        type_id: HirTypeId,
        substitutions: &HashMap<HirTypeParameterId, StructuralTypeId>,
    ) -> Option<StructuralTypeId> {
        self.lower_type(type_id, substitutions, &mut Vec::new())
            .ok()
    }

    fn lower_source_signal_payload(
        mut self,
        annotation: HirTypeId,
    ) -> Result<LoweredDecodeType, SourceDecodeElaborationBlocker> {
        let mut item_stack = Vec::new();
        let (payload_annotation, substitutions) =
            self.resolve_signal_payload(annotation, &HashMap::new(), &mut item_stack)?;
        let subject = self
            .lower_type(payload_annotation, &substitutions, &mut item_stack)
            .map_err(DecodeTypeLoweringError::into_blocker)?;
        Ok(LoweredDecodeType {
            payload_annotation,
            structural_types: self.types,
            subject,
            domain_bindings: self.domain_bindings,
        })
    }

    fn resolve_signal_payload(
        &mut self,
        annotation: HirTypeId,
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<(HirTypeId, StructuralSubstitutions), SourceDecodeElaborationBlocker> {
        let ty = self.module.types()[annotation].clone();
        match ty.kind {
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
                    return Err(SourceDecodeElaborationBlocker::AnnotationNotSignal {
                        span: ty.span,
                    });
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                        Ok((*arguments.first(), substitutions.clone()))
                    }
                    ResolutionState::Resolved(TypeResolution::Item(item_id)) => self
                        .resolve_signal_payload_item(
                            *item_id,
                            &arguments.iter().copied().collect::<Vec<_>>(),
                            substitutions,
                            item_stack,
                            ty.span,
                        ),
                    ResolutionState::Resolved(TypeResolution::Builtin(_))
                    | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
                    | ResolutionState::Resolved(TypeResolution::Import(_))
                    | ResolutionState::Unresolved => {
                        Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span: ty.span })
                    }
                }
            }
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => self
                    .resolve_signal_payload_item(*item_id, &[], substitutions, item_stack, ty.span),
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => {
                    Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span: ty.span })
                }
            },
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => {
                Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span: ty.span })
            }
        }
    }

    fn resolve_signal_payload_item(
        &mut self,
        item_id: ItemId,
        arguments: &[HirTypeId],
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
        span: SourceSpan,
    ) -> Result<(HirTypeId, StructuralSubstitutions), SourceDecodeElaborationBlocker> {
        if item_stack.contains(&item_id) {
            return Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span });
        }
        let item = self.module.items()[item_id].clone();
        let Item::Type(item) = item else {
            return Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span });
        };
        if item.parameters.len() != arguments.len() {
            return Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span });
        }
        let TypeItemBody::Alias(alias) = item.body else {
            return Err(SourceDecodeElaborationBlocker::AnnotationNotSignal { span });
        };

        let mut item_substitutions = HashMap::with_capacity(item.parameters.len());
        for (parameter, argument) in item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().copied())
        {
            item_substitutions.insert(
                parameter,
                self.lower_type(argument, substitutions, item_stack)
                    .map_err(DecodeTypeLoweringError::into_blocker)?,
            );
        }

        item_stack.push(item_id);
        let payload = self.resolve_signal_payload(alias, &item_substitutions, item_stack);
        let popped = item_stack.pop();
        debug_assert_eq!(popped, Some(item_id));
        payload
    }

    fn lower_type(
        &mut self,
        type_id: HirTypeId,
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<StructuralTypeId, DecodeTypeLoweringError> {
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => {
                self.lower_type_reference(&reference, ty.span, substitutions, item_stack)
            }
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter().copied() {
                    lowered.push(self.lower_type(element, substitutions, item_stack)?);
                }
                self.types.tuple(lowered).map_err(|error| {
                    DecodeTypeLoweringError::invalid_shape(ty.span, error.kind().clone())
                })
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(RecordField::new(
                        field.label.text(),
                        self.lower_type(field.ty, substitutions, item_stack)?,
                    ));
                }
                self.types
                    .record(Closedness::Closed, lowered)
                    .map_err(|error| {
                        DecodeTypeLoweringError::invalid_shape(ty.span, error.kind().clone())
                    })
            }
            TypeKind::Arrow { .. } => Err(DecodeTypeLoweringError::unsupported(
                ty.span,
                SourceDecodeUnsupportedTypeKind::Arrow,
            )),
            TypeKind::Apply { callee, arguments } => {
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter().copied() {
                    lowered_arguments.push(self.lower_type(argument, substitutions, item_stack)?);
                }
                self.lower_type_application(
                    callee,
                    ty.span,
                    &lowered_arguments,
                    substitutions,
                    item_stack,
                )
            }
        }
    }

    fn lower_type_reference(
        &mut self,
        reference: &crate::TypeReference,
        span: SourceSpan,
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<StructuralTypeId, DecodeTypeLoweringError> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
                match builtin_scalar(*builtin) {
                    Some(primitive) => Ok(self.types.primitive(primitive)),
                    None => match builtin {
                        BuiltinType::Signal => Err(DecodeTypeLoweringError::unsupported(
                            span,
                            SourceDecodeUnsupportedTypeKind::NestedSignal,
                        )),
                        BuiltinType::Task => Err(DecodeTypeLoweringError::unsupported(
                            span,
                            SourceDecodeUnsupportedTypeKind::Task,
                        )),
                        BuiltinType::List
                        | BuiltinType::Map
                        | BuiltinType::Set
                        | BuiltinType::Option
                        | BuiltinType::Result
                        | BuiltinType::Validation => {
                            Ok(self.external_reference(builtin_type_name(*builtin)))
                        }
                        BuiltinType::Int
                        | BuiltinType::Float
                        | BuiltinType::Decimal
                        | BuiltinType::BigInt
                        | BuiltinType::Bool
                        | BuiltinType::Text
                        | BuiltinType::Unit
                        | BuiltinType::Bytes => {
                            unreachable!("scalar builtins should have matched above")
                        }
                    },
                }
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                Ok(substitutions
                    .get(parameter)
                    .copied()
                    .unwrap_or_else(|| self.parameter_reference(*parameter)))
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Import(_)) | ResolutionState::Unresolved => {
                Ok(self.external_reference(type_path_name(&reference.path)))
            }
        }
    }

    fn lower_type_application(
        &mut self,
        callee: HirTypeId,
        span: SourceSpan,
        arguments: &[StructuralTypeId],
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<StructuralTypeId, DecodeTypeLoweringError> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return Ok(self.external_reference("<applied-type>"));
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Ok(self.types.list(
                    *arguments
                        .first()
                        .expect("validated List arity should be one"),
                ))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                Ok(self.types.option(
                    *arguments
                        .first()
                        .expect("validated Option arity should be one"),
                ))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                Ok(self.types.result(
                    *arguments
                        .first()
                        .expect("validated Result error arity should exist"),
                    *arguments
                        .get(1)
                        .expect("validated Result value arity should exist"),
                ))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                Ok(self.types.validation(
                    *arguments
                        .first()
                        .expect("validated Validation error arity should exist"),
                    *arguments
                        .get(1)
                        .expect("validated Validation value arity should exist"),
                ))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                Err(DecodeTypeLoweringError::unsupported(
                    span,
                    SourceDecodeUnsupportedTypeKind::NestedSignal,
                ))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => Err(
                DecodeTypeLoweringError::unsupported(span, SourceDecodeUnsupportedTypeKind::Task),
            ),
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
                Ok(self.external_reference(builtin_type_name(*builtin)))
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, arguments, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                Ok(substitutions
                    .get(parameter)
                    .copied()
                    .unwrap_or_else(|| self.parameter_reference(*parameter)))
            }
            ResolutionState::Resolved(TypeResolution::Import(_)) | ResolutionState::Unresolved => {
                Ok(self.external_reference(type_path_name(&reference.path)))
            }
        }
    }

    fn lower_type_item(
        &mut self,
        item_id: ItemId,
        arguments: &[StructuralTypeId],
        item_stack: &mut Vec<ItemId>,
    ) -> Result<StructuralTypeId, DecodeTypeLoweringError> {
        if item_stack.contains(&item_id) {
            return Ok(self.external_reference(item_type_name(&self.module.items()[item_id])));
        }
        let item = self.module.items()[item_id].clone();
        match item {
            Item::Type(item) => {
                if item.parameters.len() != arguments.len() {
                    return Ok(self.external_reference(item.name.text()));
                }
                let item_substitutions =
                    combine_item_substitutions(item.parameters.iter().copied(), arguments);
                item_stack.push(item_id);
                let lowered = match item.body {
                    TypeItemBody::Alias(alias) => {
                        self.lower_type(alias, &item_substitutions, item_stack)
                    }
                    TypeItemBody::Sum(variants) => {
                        let variants = variants
                            .iter()
                            .map(|variant| {
                                self.lower_sum_variant(variant, &item_substitutions, item_stack)
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        self.types
                            .sum(Closedness::Closed, variants)
                            .map_err(|error| {
                                DecodeTypeLoweringError::invalid_shape(
                                    item.header.span,
                                    error.kind().clone(),
                                )
                            })
                    }
                };
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(item_id));
                lowered
            }
            Item::Domain(item) => {
                if item.parameters.len() != arguments.len() {
                    return Ok(self.external_reference(item.name.text()));
                }
                let item_substitutions =
                    combine_item_substitutions(item.parameters.iter().copied(), arguments);
                item_stack.push(item_id);
                let carrier = self.lower_type(item.carrier, &item_substitutions, item_stack);
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(item_id));
                let ty = self.types.domain(item.name.text(), carrier?);
                self.domain_bindings.push(SourceDecodeDomainBinding {
                    ty,
                    domain_item: item_id,
                    arguments: arguments.to_vec(),
                });
                Ok(ty)
            }
            Item::Class(_)
            | Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => Ok(self.external_reference(item_type_name(&item))),
        }
    }

    fn lower_sum_variant(
        &mut self,
        variant: &TypeVariant,
        substitutions: &StructuralSubstitutions,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<SumVariant, DecodeTypeLoweringError> {
        let payload = match variant.fields.as_slice() {
            [] => None,
            [only] => Some(self.lower_type(*only, substitutions, item_stack)?),
            many => {
                let mut lowered = Vec::with_capacity(many.len());
                for field in many.iter().copied() {
                    lowered.push(self.lower_type(field, substitutions, item_stack)?);
                }
                Some(self.types.tuple(lowered).map_err(|error| {
                    DecodeTypeLoweringError::invalid_shape(variant.span, error.kind().clone())
                })?)
            }
        };
        Ok(match payload {
            Some(payload) => SumVariant::unary(variant.name.text(), payload),
            None => SumVariant::nullary(variant.name.text()),
        })
    }

    fn parameter_reference(&mut self, parameter: HirTypeParameterId) -> StructuralTypeId {
        let structural = if let Some(parameter_id) = self.parameters.get(&parameter).copied() {
            parameter_id
        } else {
            let name = self.module.type_parameters()[parameter]
                .name
                .text()
                .to_owned();
            let parameter_id = self.types.define_parameter(name);
            self.parameters.insert(parameter, parameter_id);
            parameter_id
        };
        self.types.parameter(structural)
    }

    fn external_reference(&mut self, name: impl Into<String>) -> StructuralTypeId {
        let name = name.into();
        let external = if let Some(external) = self.externals.get(&name).copied() {
            external
        } else {
            let external = self.types.define_external(name.clone());
            self.externals.insert(name, external);
            external
        };
        self.types.external(external)
    }
}

type StructuralSubstitutions = HashMap<HirTypeParameterId, StructuralTypeId>;

fn combine_item_substitutions(
    parameters: impl Iterator<Item = HirTypeParameterId>,
    arguments: &[StructuralTypeId],
) -> StructuralSubstitutions {
    parameters.zip(arguments.iter().copied()).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodeTypeLoweringError {
    span: SourceSpan,
    kind: DecodeTypeLoweringErrorKind,
}

impl DecodeTypeLoweringError {
    fn unsupported(span: SourceSpan, kind: SourceDecodeUnsupportedTypeKind) -> Self {
        Self {
            span,
            kind: DecodeTypeLoweringErrorKind::Unsupported(kind),
        }
    }

    fn invalid_shape(span: SourceSpan, kind: ShapeErrorKind) -> Self {
        Self {
            span,
            kind: DecodeTypeLoweringErrorKind::InvalidShape(kind),
        }
    }

    fn into_blocker(self) -> SourceDecodeElaborationBlocker {
        match self.kind {
            DecodeTypeLoweringErrorKind::Unsupported(kind) => {
                SourceDecodeElaborationBlocker::UnsupportedPayloadType {
                    span: self.span,
                    kind,
                }
            }
            DecodeTypeLoweringErrorKind::InvalidShape(kind) => {
                SourceDecodeElaborationBlocker::InvalidPayloadShape {
                    span: self.span,
                    kind,
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DecodeTypeLoweringErrorKind {
    Unsupported(SourceDecodeUnsupportedTypeKind),
    InvalidShape(ShapeErrorKind),
}

fn builtin_scalar(builtin: BuiltinType) -> Option<PrimitiveType> {
    match builtin {
        BuiltinType::Int => Some(PrimitiveType::Int),
        BuiltinType::Float => Some(PrimitiveType::Float),
        BuiltinType::Decimal => Some(PrimitiveType::Decimal),
        BuiltinType::BigInt => Some(PrimitiveType::BigInt),
        BuiltinType::Bool => Some(PrimitiveType::Bool),
        BuiltinType::Text => Some(PrimitiveType::Text),
        BuiltinType::Unit => Some(PrimitiveType::Unit),
        BuiltinType::Bytes => Some(PrimitiveType::Bytes),
        BuiltinType::List
        | BuiltinType::Map
        | BuiltinType::Set
        | BuiltinType::Option
        | BuiltinType::Result
        | BuiltinType::Validation
        | BuiltinType::Signal
        | BuiltinType::Task => None,
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

fn type_path_name(path: &crate::NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
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

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;
    use aivi_typing::{
        DecodeDomainRule, DecodeExtraFieldPolicy, DecodeMode, DecodePlanningErrorKind, DecodeStep,
    };

    use super::{
        SourceDecodeElaborationBlocker, SourceDecodeNodeOutcome, SourceDecodeUnsupportedTypeKind,
        elaborate_source_decodes,
    };
    use crate::{Item, lower_module};

    fn lower_text(path: &str, text: &str) -> crate::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn item_name(module: &crate::Module, item_id: crate::ItemId) -> &str {
        match &module.items()[item_id] {
            Item::Type(item) => item.name.text(),
            Item::Value(item) => item.name.text(),
            Item::Function(item) => item.name.text(),
            Item::Signal(item) => item.name.text(),
            Item::Class(item) => item.name.text(),
            Item::Domain(item) => item.name.text(),
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => "<anonymous>",
        }
    }

    #[test]
    fn elaborates_result_payload_decode_schema_for_source_signals() {
        let lowered = lower_text(
            "source_decode_result.aivi",
            r#"
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

@source custom.feed
sig users : Signal (Result HttpError (List User))
"#,
        );
        assert!(
            !lowered.has_errors(),
            "source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let users = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "users")
            .expect("expected decode plan for users");

        match &users.outcome {
            SourceDecodeNodeOutcome::Planned(plan) => {
                assert_eq!(plan.mode, DecodeMode::Strict);
                let DecodeStep::Result { error, value, .. } = plan.schema.root_step() else {
                    panic!("expected result-shaped published payload");
                };
                assert!(matches!(plan.schema.step(*error), DecodeStep::Sum { .. }));
                let DecodeStep::List { element, .. } = plan.schema.step(*value) else {
                    panic!("expected result success payload to stay a list");
                };
                assert!(matches!(
                    plan.schema.step(*element),
                    DecodeStep::Record { .. }
                ));
            }
            other => panic!("expected planned decode node, found {other:?}"),
        }
    }

    #[test]
    fn permissive_decode_mode_relaxes_record_extra_field_policy() {
        let lowered = lower_text(
            "source_decode_permissive.aivi",
            r#"
type DecodeMode =
  | Strict
  | Permissive

type User = {
    id: Int
}

val mode = Permissive

@source custom.feed with {
    decode: mode
}
sig user : Signal User
"#,
        );
        assert!(
            !lowered.has_errors(),
            "permissive source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let user = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "user")
            .expect("expected decode plan for user");

        match &user.outcome {
            SourceDecodeNodeOutcome::Planned(plan) => {
                assert_eq!(plan.mode, DecodeMode::Permissive);
                let DecodeStep::Record { extra_fields, .. } = plan.schema.root_step() else {
                    panic!("expected direct record payload");
                };
                assert_eq!(*extra_fields, DecodeExtraFieldPolicy::Ignore);
            }
            other => panic!("expected planned decode node, found {other:?}"),
        }
    }

    #[test]
    fn multi_field_sum_variants_lower_to_tuple_payloads() {
        let lowered = lower_text(
            "source_decode_tuple_payload.aivi",
            r#"
type Message =
  | Ping
  | Data Text Int

@source custom.feed
sig inbox : Signal Message
"#,
        );
        assert!(
            !lowered.has_errors(),
            "tuple payload source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let inbox = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "inbox")
            .expect("expected decode plan for inbox");

        match &inbox.outcome {
            SourceDecodeNodeOutcome::Planned(plan) => {
                let DecodeStep::Sum { variants, .. } = plan.schema.root_step() else {
                    panic!("expected source payload to preserve the underlying sum");
                };
                let payload = variants[1]
                    .payload
                    .expect("multi-field constructor should lower to one tuple payload schema");
                assert!(matches!(
                    plan.schema.step(payload),
                    DecodeStep::Tuple { .. }
                ));
            }
            other => panic!("expected planned decode node, found {other:?}"),
        }
    }

    #[test]
    fn domain_payloads_preserve_explicit_domain_handoff() {
        let lowered = lower_text(
            "source_decode_domain.aivi",
            r#"
domain Url over Text
    parse : Text -> Result Text Url

@source custom.feed
sig url : Signal Url
"#,
        );
        assert!(
            !lowered.has_errors(),
            "domain source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let url = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "url")
            .expect("expected decode plan for url");

        match &url.outcome {
            SourceDecodeNodeOutcome::Planned(plan) => {
                let DecodeStep::Domain { rule, .. } = plan.schema.root_step() else {
                    panic!("expected domain payload to stay nominal");
                };
                assert_eq!(*rule, DecodeDomainRule::ExplicitSurface);
            }
            other => panic!("expected planned decode node, found {other:?}"),
        }
    }

    #[test]
    fn blocks_nested_signal_payloads() {
        let lowered = lower_text(
            "source_decode_nested_signal.aivi",
            r#"
@source custom.feed
sig bad : Signal (Signal Int)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "nested signal source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let bad = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "bad")
            .expect("expected blocked decode node for bad");

        match &bad.outcome {
            SourceDecodeNodeOutcome::Blocked(node) => {
                assert!(
                    node.blockers.contains(
                        &SourceDecodeElaborationBlocker::UnsupportedPayloadType {
                            span: lowered.module().items()[bad.owner].span(),
                            kind: SourceDecodeUnsupportedTypeKind::NestedSignal,
                        }
                    ) || node.blockers.iter().any(|blocker| {
                        matches!(
                            blocker,
                            SourceDecodeElaborationBlocker::UnsupportedPayloadType {
                                kind: SourceDecodeUnsupportedTypeKind::NestedSignal,
                                ..
                            }
                        )
                    })
                );
            }
            other => panic!("expected blocked decode node, found {other:?}"),
        }
    }

    #[test]
    fn blocks_recursive_payloads_until_explicit_recursive_decoder_support_exists() {
        let lowered = lower_text(
            "source_decode_recursive_tree.aivi",
            r#"
type Tree =
  | Leaf
  | Branch Tree Tree

@source custom.feed
sig tree : Signal Tree
"#,
        );
        assert!(
            !lowered.has_errors(),
            "recursive source decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_source_decodes(lowered.module());
        let tree = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "tree")
            .expect("expected blocked decode node for tree");

        match &tree.outcome {
            SourceDecodeNodeOutcome::Blocked(node) => {
                let planning = node
                    .blockers
                    .iter()
                    .find_map(|blocker| match blocker {
                        SourceDecodeElaborationBlocker::DecodePlanning(error) => Some(error),
                        _ => None,
                    })
                    .expect("expected recursive payload to block in decode planning");
                assert!(matches!(
                    planning.kind(),
                    DecodePlanningErrorKind::OpaqueReference { .. }
                ));
            }
            other => panic!("expected blocked decode node, found {other:?}"),
        }
    }
}
