use std::collections::HashMap;

use aivi_base::SourceSpan;
use aivi_typing::{
    DecodeExtraFieldPolicy, DecodeFieldRequirement, DecodeMode, DecodePlanId, DecodeStep,
    DecodeSumStrategy, FieldName, PrimitiveType, TypeId as StructuralTypeId, TypeNode,
    TypeReference, VariantName,
};

use crate::{
    DomainMemberKind, Item, ItemId, Module, SourceDecodeElaborationBlocker,
    SourceDecodeNodeOutcome, SourceDecodePlan, TypeKind, TypeParameterId as HirTypeParameterId,
    decode_elaboration::{DecodeTypeLowerer, elaborate_source_decodes},
};

/// Concrete pre-runtime structural decoder programs derived from planned source-decode schemas.
///
/// This stays intentionally above runtime execution: it resolves only the deterministic compiler
/// choices the current frontend can already justify today:
/// - the ordered decoder walk for closed structural shapes,
/// - explicit field/variant/container assembly,
/// - and the chosen explicit domain surface when a same-module domain exposes one.
///
/// Wire-format decisions, scheduler publication, and typed source-error transport remain later
/// runtime/backend work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceDecodeProgramReport {
    nodes: Vec<SourceDecodeProgramNode>,
}

impl SourceDecodeProgramReport {
    pub fn new(nodes: Vec<SourceDecodeProgramNode>) -> Self {
        Self { nodes }
    }

    pub fn nodes(&self) -> &[SourceDecodeProgramNode] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<SourceDecodeProgramNode> {
        self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecodeProgramNode {
    pub owner: ItemId,
    pub source_span: SourceSpan,
    pub outcome: SourceDecodeProgramOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeProgramOutcome {
    Planned(SourceDecodeProgram),
    Blocked(BlockedSourceDecodeProgram),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct DecodeProgramStepId(u32);

impl DecodeProgramStepId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("decode program table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecodeProgram {
    pub mode: DecodeMode,
    pub payload_annotation: crate::TypeId,
    root: DecodeProgramStepId,
    steps: Vec<DecodeProgramStep>,
}

impl SourceDecodeProgram {
    pub fn root(&self) -> DecodeProgramStepId {
        self.root
    }

    pub fn root_step(&self) -> &DecodeProgramStep {
        self.step(self.root)
    }

    pub fn step(&self, id: DecodeProgramStepId) -> &DecodeProgramStep {
        &self.steps[id.index()]
    }

    pub fn steps(&self) -> &[DecodeProgramStep] {
        &self.steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedSourceDecodeProgram {
    pub decode: Option<SourceDecodePlan>,
    pub blockers: Vec<SourceDecodeProgramBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceDecodeProgramBlocker {
    DecodeElaboration(SourceDecodeElaborationBlocker),
    MissingDomainBinding {
        domain_name: Box<str>,
    },
    MissingDomainSurface {
        domain_item: ItemId,
        domain_name: Box<str>,
    },
    AmbiguousDomainSurface {
        domain_item: ItemId,
        domain_name: Box<str>,
        candidates: Vec<DomainDecodeSurfaceCandidate>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeProgramStep {
    Scalar {
        scalar: PrimitiveType,
    },
    Tuple {
        elements: Vec<DecodeProgramStepId>,
    },
    Record {
        fields: Vec<DecodeProgramField>,
        extra_fields: DecodeExtraFieldPolicy,
    },
    Sum {
        variants: Vec<DecodeProgramVariant>,
        strategy: DecodeSumStrategy,
    },
    Domain {
        carrier: DecodeProgramStepId,
        surface: DomainDecodeSurfacePlan,
    },
    List {
        element: DecodeProgramStepId,
    },
    Option {
        element: DecodeProgramStepId,
    },
    Result {
        error: DecodeProgramStepId,
        value: DecodeProgramStepId,
    },
    Validation {
        error: DecodeProgramStepId,
        value: DecodeProgramStepId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeProgramField {
    pub name: FieldName,
    pub requirement: DecodeFieldRequirement,
    pub step: DecodeProgramStepId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeProgramVariant {
    pub constructor: Option<crate::SumConstructorHandle>,
    pub name: VariantName,
    pub payload: Option<DecodeProgramStepId>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DomainDecodeSurfaceKind {
    Direct,
    FallibleResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainDecodeSurfacePlan {
    pub domain_item: ItemId,
    pub member_index: usize,
    pub member_name: Box<str>,
    pub kind: DomainDecodeSurfaceKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainDecodeSurfaceCandidate {
    pub member_index: usize,
    pub member_name: Box<str>,
    pub kind: DomainDecodeSurfaceKind,
    pub span: SourceSpan,
}

pub fn generate_source_decode_programs(module: &Module) -> SourceDecodeProgramReport {
    let decode_report = elaborate_source_decodes(module);
    let mut nodes = Vec::with_capacity(decode_report.nodes().len());

    for node in decode_report.into_nodes() {
        let outcome = match node.outcome {
            SourceDecodeNodeOutcome::Planned(plan) => {
                match generate_source_decode_program(module, &plan) {
                    Ok(program) => SourceDecodeProgramOutcome::Planned(program),
                    Err(blockers) => {
                        SourceDecodeProgramOutcome::Blocked(BlockedSourceDecodeProgram {
                            decode: Some(plan),
                            blockers,
                        })
                    }
                }
            }
            SourceDecodeNodeOutcome::Blocked(blocked) => {
                SourceDecodeProgramOutcome::Blocked(BlockedSourceDecodeProgram {
                    decode: None,
                    blockers: blocked
                        .blockers
                        .into_iter()
                        .map(SourceDecodeProgramBlocker::DecodeElaboration)
                        .collect(),
                })
            }
        };
        nodes.push(SourceDecodeProgramNode {
            owner: node.owner,
            source_span: node.source_span,
            outcome,
        });
    }

    SourceDecodeProgramReport::new(nodes)
}

fn generate_source_decode_program(
    module: &Module,
    plan: &SourceDecodePlan,
) -> Result<SourceDecodeProgram, Vec<SourceDecodeProgramBlocker>> {
    let surfaces = resolve_domain_surfaces(module, plan)?;
    let mut steps = Vec::with_capacity(plan.schema.steps().len());

    for step in plan.schema.steps() {
        steps.push(match step {
            DecodeStep::IntrinsicScalar { scalar, .. } => {
                DecodeProgramStep::Scalar { scalar: *scalar }
            }
            DecodeStep::Tuple { elements, .. } => DecodeProgramStep::Tuple {
                elements: elements.iter().copied().map(program_step_id).collect(),
            },
            DecodeStep::Record {
                fields,
                extra_fields,
                ..
            } => DecodeProgramStep::Record {
                fields: fields
                    .iter()
                    .map(|field| DecodeProgramField {
                        name: field.name.clone(),
                        requirement: field.requirement,
                        step: program_step_id(field.schema),
                    })
                    .collect(),
                extra_fields: *extra_fields,
            },
            DecodeStep::Sum {
                ty,
                variants,
                strategy,
                ..
            } => {
                let constructor_type_item = plan.sum_binding(*ty).map(|binding| binding.type_item);
                DecodeProgramStep::Sum {
                    variants: variants
                        .iter()
                        .map(|variant| DecodeProgramVariant {
                            constructor: constructor_type_item.and_then(|type_item| {
                                module.sum_constructor_handle(type_item, variant.name.as_str())
                            }),
                            name: variant.name.clone(),
                            payload: variant.payload.map(program_step_id),
                        })
                        .collect(),
                    strategy: *strategy,
                }
            }
            DecodeStep::Domain { ty, carrier, .. } => DecodeProgramStep::Domain {
                carrier: program_step_id(*carrier),
                surface: surfaces
                    .get(ty)
                    .cloned()
                    .expect("domain surfaces should be resolved before program generation"),
            },
            DecodeStep::List { element, .. } => DecodeProgramStep::List {
                element: program_step_id(*element),
            },
            DecodeStep::Option { element, .. } => DecodeProgramStep::Option {
                element: program_step_id(*element),
            },
            DecodeStep::Result { error, value, .. } => DecodeProgramStep::Result {
                error: program_step_id(*error),
                value: program_step_id(*value),
            },
            DecodeStep::Validation { error, value, .. } => DecodeProgramStep::Validation {
                error: program_step_id(*error),
                value: program_step_id(*value),
            },
        });
    }

    Ok(SourceDecodeProgram {
        mode: plan.mode,
        payload_annotation: plan.payload_annotation,
        root: program_step_id(plan.schema.root()),
        steps,
    })
}

fn resolve_domain_surfaces(
    module: &Module,
    plan: &SourceDecodePlan,
) -> Result<HashMap<StructuralTypeId, DomainDecodeSurfacePlan>, Vec<SourceDecodeProgramBlocker>> {
    let mut surfaces = HashMap::new();
    let mut blockers = Vec::new();

    for step in plan.schema.steps() {
        let DecodeStep::Domain { ty, carrier, .. } = step else {
            continue;
        };
        let carrier_ty = plan.schema.step(*carrier).ty();
        match resolve_domain_surface(module, plan, *ty, carrier_ty) {
            Ok(surface) => {
                surfaces.insert(*ty, surface);
            }
            Err(blocker) => blockers.push(blocker),
        }
    }

    if blockers.is_empty() {
        Ok(surfaces)
    } else {
        Err(blockers)
    }
}

fn resolve_domain_surface(
    module: &Module,
    plan: &SourceDecodePlan,
    domain_ty: StructuralTypeId,
    carrier_ty: StructuralTypeId,
) -> Result<DomainDecodeSurfacePlan, SourceDecodeProgramBlocker> {
    let domain_name = domain_type_name(&plan.structural_types, domain_ty);
    let binding = plan.domain_binding(domain_ty).ok_or_else(|| {
        SourceDecodeProgramBlocker::MissingDomainBinding {
            domain_name: domain_name.clone().into_boxed_str(),
        }
    })?;
    let Item::Domain(domain) = &module.items()[binding.domain_item] else {
        return Err(SourceDecodeProgramBlocker::MissingDomainBinding {
            domain_name: domain_name.into_boxed_str(),
        });
    };

    let substitutions = domain
        .parameters
        .iter()
        .copied()
        .zip(binding.arguments.iter().copied())
        .collect::<HashMap<HirTypeParameterId, StructuralTypeId>>();
    let mut lowerer = DecodeTypeLowerer::with_type_store(module, plan.structural_types.clone());
    let mut candidates = Vec::new();

    for (member_index, member) in domain.members.iter().enumerate() {
        if member.kind != DomainMemberKind::Method {
            continue;
        }
        let TypeKind::Arrow { parameter, result } = &module.types()[member.annotation].kind else {
            continue;
        };
        let Some(parameter_ty) = lowerer.lower_type_with_substitutions(*parameter, &substitutions)
        else {
            continue;
        };
        if !structural_type_eq(lowerer.types(), parameter_ty, carrier_ty) {
            continue;
        }
        let Some(result_ty) = lowerer.lower_type_with_substitutions(*result, &substitutions) else {
            continue;
        };
        let Some(kind) = classify_domain_surface(lowerer.types(), result_ty, domain_ty) else {
            continue;
        };
        candidates.push(DomainDecodeSurfaceCandidate {
            member_index,
            member_name: member.name.text().into(),
            kind,
            span: member.span,
        });
    }

    let parse_candidates = candidates
        .iter()
        .filter(|candidate| {
            candidate.member_name.as_ref() == "parse"
                && matches!(candidate.kind, DomainDecodeSurfaceKind::FallibleResult)
        })
        .cloned()
        .collect::<Vec<_>>();
    let selected = match parse_candidates.as_slice() {
        [candidate] => Some(candidate.clone()),
        [] => match candidates.as_slice() {
            [candidate] => Some(candidate.clone()),
            _ => None,
        },
        _ => None,
    };

    if let Some(candidate) = selected {
        return Ok(DomainDecodeSurfacePlan {
            domain_item: binding.domain_item,
            member_index: candidate.member_index,
            member_name: candidate.member_name,
            kind: candidate.kind,
            span: candidate.span,
        });
    }

    if candidates.is_empty() {
        Err(SourceDecodeProgramBlocker::MissingDomainSurface {
            domain_item: binding.domain_item,
            domain_name: domain.name.text().into(),
        })
    } else {
        Err(SourceDecodeProgramBlocker::AmbiguousDomainSurface {
            domain_item: binding.domain_item,
            domain_name: domain.name.text().into(),
            candidates,
        })
    }
}

fn classify_domain_surface(
    types: &aivi_typing::TypeStore,
    result_ty: StructuralTypeId,
    domain_ty: StructuralTypeId,
) -> Option<DomainDecodeSurfaceKind> {
    if structural_type_eq(types, result_ty, domain_ty) {
        return Some(DomainDecodeSurfaceKind::Direct);
    }
    let TypeNode::Result { value, .. } = types.node(result_ty) else {
        return None;
    };
    structural_type_eq(types, *value, domain_ty).then_some(DomainDecodeSurfaceKind::FallibleResult)
}

fn structural_type_eq(
    types: &aivi_typing::TypeStore,
    left: StructuralTypeId,
    right: StructuralTypeId,
) -> bool {
    let mut stack = vec![(left, right)];
    while let Some((left, right)) = stack.pop() {
        if left == right {
            continue;
        }
        match (types.node(left), types.node(right)) {
            (TypeNode::Primitive(left), TypeNode::Primitive(right)) if left == right => {}
            (
                TypeNode::Reference(TypeReference::Parameter(left)),
                TypeNode::Reference(TypeReference::Parameter(right)),
            ) if types.parameter_name(*left) == types.parameter_name(*right) => {}
            (
                TypeNode::Reference(TypeReference::External(left)),
                TypeNode::Reference(TypeReference::External(right)),
            ) if types.external_name(*left) == types.external_name(*right) => {}
            (TypeNode::Tuple(left), TypeNode::Tuple(right)) if left.len() == right.len() => {
                stack.extend(left.iter().copied().zip(right.iter().copied()));
            }
            (TypeNode::Record(left), TypeNode::Record(right))
                if left.closedness() == right.closedness()
                    && left.fields().len() == right.fields().len() =>
            {
                for (left_field, right_field) in left.fields().iter().zip(right.fields()) {
                    if left_field.name() != right_field.name() {
                        return false;
                    }
                    stack.push((left_field.ty(), right_field.ty()));
                }
            }
            (TypeNode::Sum(left), TypeNode::Sum(right))
                if left.closedness() == right.closedness()
                    && left.variants().len() == right.variants().len() =>
            {
                for (left_variant, right_variant) in left.variants().iter().zip(right.variants()) {
                    if left_variant.name() != right_variant.name() {
                        return false;
                    }
                    match (left_variant.payload(), right_variant.payload()) {
                        (Some(left), Some(right)) => stack.push((left, right)),
                        (None, None) => {}
                        _ => return false,
                    }
                }
            }
            (TypeNode::Domain(left), TypeNode::Domain(right)) if left.name() == right.name() => {
                stack.push((left.carrier(), right.carrier()));
            }
            (TypeNode::List(left), TypeNode::List(right))
            | (TypeNode::Option(left), TypeNode::Option(right)) => {
                stack.push((*left, *right));
            }
            (
                TypeNode::Result {
                    error: left_error,
                    value: left_value,
                },
                TypeNode::Result {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                TypeNode::Validation {
                    error: left_error,
                    value: left_value,
                },
                TypeNode::Validation {
                    error: right_error,
                    value: right_value,
                },
            ) => {
                stack.push((*left_error, *right_error));
                stack.push((*left_value, *right_value));
            }
            _ => return false,
        }
    }
    true
}

fn domain_type_name(types: &aivi_typing::TypeStore, ty: StructuralTypeId) -> String {
    match types.node(ty) {
        TypeNode::Domain(domain) => domain.name().to_owned(),
        _ => "<domain>".to_owned(),
    }
}

fn program_step_id(id: DecodePlanId) -> DecodeProgramStepId {
    DecodeProgramStepId::from_index(id.as_usize())
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{
        DomainDecodeSurfaceKind, SourceDecodeProgramBlocker, SourceDecodeProgramOutcome,
        generate_source_decode_programs,
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
    fn generates_nested_domain_surface_steps_for_structural_payloads() {
        let lowered = lower_text(
            "source_decode_program_nested_domain.aivi",
            r#"
type HttpError =
  | Timeout

domain Url over Text
    parse : Text -> Result Text Url
    unwrap : Url -> Text

type User = {
    id: Int,
    home: Url
}

@source custom.feed
signal users : Signal (Result HttpError (List User))
"#,
        );
        assert!(
            !lowered.has_errors(),
            "nested-domain decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let users = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "users")
            .expect("expected decode program for users");

        match &users.outcome {
            SourceDecodeProgramOutcome::Planned(program) => {
                let crate::DecodeProgramStep::Result { value, .. } = program.root_step() else {
                    panic!("expected result-shaped decoder root");
                };
                let crate::DecodeProgramStep::List { element } = program.step(*value) else {
                    panic!("expected result success branch to stay a list");
                };
                let crate::DecodeProgramStep::Record { fields, .. } = program.step(*element) else {
                    panic!("expected list element decoder to stay record-shaped");
                };
                let home = fields
                    .iter()
                    .find(|field| field.name.as_str() == "home")
                    .expect("expected record field for nested domain");
                let crate::DecodeProgramStep::Domain { surface, .. } = program.step(home.step)
                else {
                    panic!("expected nested domain field to lower through explicit surface");
                };
                assert_eq!(surface.member_name.as_ref(), "parse");
                assert_eq!(surface.kind, DomainDecodeSurfaceKind::FallibleResult);
            }
            other => panic!("expected planned decode program, found {other:?}"),
        }
    }

    #[test]
    fn prefers_parse_member_when_multiple_domain_surfaces_exist() {
        let lowered = lower_text(
            "source_decode_program_parse_preference.aivi",
            r#"
domain Duration over Int
    parse : Int -> Result Text Duration
    millis : Int -> Duration
    unwrap : Duration -> Int

@source custom.feed
signal timeout : Signal Duration
"#,
        );
        assert!(
            !lowered.has_errors(),
            "parse-preference decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let timeout = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "timeout")
            .expect("expected decode program for timeout");

        match &timeout.outcome {
            SourceDecodeProgramOutcome::Planned(program) => {
                let crate::DecodeProgramStep::Domain { surface, .. } = program.root_step() else {
                    panic!("expected domain decoder root");
                };
                assert_eq!(surface.member_name.as_ref(), "parse");
                assert_eq!(surface.kind, DomainDecodeSurfaceKind::FallibleResult);
            }
            other => panic!("expected planned decode program, found {other:?}"),
        }
    }

    #[test]
    fn uses_single_direct_constructor_surface_when_no_parse_exists() {
        let lowered = lower_text(
            "source_decode_program_direct_domain.aivi",
            r#"
domain Duration over Int
    millis : Int -> Duration
    unwrap : Duration -> Int

@source custom.feed
signal timeout : Signal Duration
"#,
        );
        assert!(
            !lowered.has_errors(),
            "direct-domain decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let timeout = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "timeout")
            .expect("expected decode program for timeout");

        match &timeout.outcome {
            SourceDecodeProgramOutcome::Planned(program) => {
                let crate::DecodeProgramStep::Domain { surface, .. } = program.root_step() else {
                    panic!("expected domain decoder root");
                };
                assert_eq!(surface.member_name.as_ref(), "millis");
                assert_eq!(surface.kind, DomainDecodeSurfaceKind::Direct);
            }
            other => panic!("expected planned decode program, found {other:?}"),
        }
    }

    #[test]
    fn blocks_ambiguous_domain_surfaces_without_parse() {
        let lowered = lower_text(
            "source_decode_program_ambiguous_domain.aivi",
            r#"
domain Duration over Int
    millis : Int -> Duration
    tryMillis : Int -> Result Text Duration
    unwrap : Duration -> Int

@source custom.feed
signal timeout : Signal Duration
"#,
        );
        assert!(
            !lowered.has_errors(),
            "ambiguous-domain decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let timeout = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "timeout")
            .expect("expected blocked decode program for timeout");

        match &timeout.outcome {
            SourceDecodeProgramOutcome::Blocked(blocked) => {
                let ambiguous = blocked
                    .blockers
                    .iter()
                    .find_map(|blocker| match blocker {
                        SourceDecodeProgramBlocker::AmbiguousDomainSurface {
                            candidates, ..
                        } => Some(candidates),
                        _ => None,
                    })
                    .expect("expected ambiguous domain surface blocker");
                assert_eq!(ambiguous.len(), 2);
            }
            other => panic!("expected blocked decode program, found {other:?}"),
        }
    }

    #[test]
    fn supports_generic_domain_surface_substitutions() {
        let lowered = lower_text(
            "source_decode_program_generic_domain.aivi",
            r#"
domain NonEmpty A over List A
    parse : List A -> Result Text (NonEmpty A)
    unwrap : NonEmpty A -> List A

@source custom.feed
signal names : Signal (NonEmpty Text)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic-domain decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let names = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "names")
            .expect("expected decode program for names");

        match &names.outcome {
            SourceDecodeProgramOutcome::Planned(program) => {
                let crate::DecodeProgramStep::Domain { surface, .. } = program.root_step() else {
                    panic!("expected domain decoder root");
                };
                assert_eq!(surface.member_name.as_ref(), "parse");
                assert_eq!(surface.kind, DomainDecodeSurfaceKind::FallibleResult);
            }
            other => panic!("expected planned decode program, found {other:?}"),
        }
    }

    #[test]
    fn forwards_decode_elaboration_blockers() {
        let lowered = lower_text(
            "source_decode_program_blocked_payload.aivi",
            r#"
@source custom.feed
signal bad : Signal (Signal Int)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "blocked decoder fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = generate_source_decode_programs(lowered.module());
        let bad = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "bad")
            .expect("expected blocked decode program for bad");

        match &bad.outcome {
            SourceDecodeProgramOutcome::Blocked(blocked) => {
                assert!(blocked.blockers.iter().any(|blocker| matches!(
                    blocker,
                    SourceDecodeProgramBlocker::DecodeElaboration(_)
                )));
                assert!(blocked.decode.is_none());
            }
            other => panic!("expected blocked decode program, found {other:?}"),
        }
    }
}
