//! Focused RFC §14.2 structural decode-schema planning.
//!
//! This module stays intentionally pre-runtime. It answers only the type-side questions the
//! current compiler wave can already prove:
//! - which closed structural shapes a builtin decoder would need to walk,
//! - which global decode mode applies to record extra-field handling,
//! - and where the current built-in path must stop instead of inventing decoder overrides.
//!
//! It does not choose wire representations, execute decoding, or resolve domain-owned parser
//! members. Those remain later work once typed core / runtime decoder generation exists.

use std::{error::Error, fmt};

use crate::eq::{
    Closedness, FieldName, PrimitiveType, TypeId, TypeNode, TypeReference, TypeStore, VariantName,
};

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum DecodeMode {
    #[default]
    Strict,
    Permissive,
}

impl DecodeMode {
    pub const fn extra_fields(self) -> DecodeExtraFieldPolicy {
        match self {
            Self::Strict => DecodeExtraFieldPolicy::Reject,
            Self::Permissive => DecodeExtraFieldPolicy::Ignore,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DecodePlanId(u32);

impl DecodePlanId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("decode plan table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }

    pub fn as_usize(self) -> usize {
        self.index()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeSchema {
    subject: TypeId,
    mode: DecodeMode,
    root: DecodePlanId,
    steps: Vec<DecodeStep>,
}

impl DecodeSchema {
    pub fn subject(&self) -> TypeId {
        self.subject
    }

    pub fn mode(&self) -> DecodeMode {
        self.mode
    }

    pub fn root(&self) -> DecodePlanId {
        self.root
    }

    pub fn root_step(&self) -> &DecodeStep {
        self.step(self.root)
    }

    pub fn step(&self, id: DecodePlanId) -> &DecodeStep {
        &self.steps[id.index()]
    }

    pub fn steps(&self) -> &[DecodeStep] {
        &self.steps
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DecodeExtraFieldPolicy {
    Reject,
    Ignore,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DecodeFieldRequirement {
    Required,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DecodeSumStrategy {
    Explicit,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DecodeDomainRule {
    ExplicitSurface,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeStep {
    IntrinsicScalar {
        ty: TypeId,
        scalar: PrimitiveType,
    },
    Tuple {
        ty: TypeId,
        elements: Vec<DecodePlanId>,
    },
    Record {
        ty: TypeId,
        fields: Vec<DecodeFieldPlan>,
        extra_fields: DecodeExtraFieldPolicy,
    },
    Sum {
        ty: TypeId,
        variants: Vec<DecodeVariantPlan>,
        strategy: DecodeSumStrategy,
    },
    Domain {
        ty: TypeId,
        carrier: DecodePlanId,
        rule: DecodeDomainRule,
    },
    List {
        ty: TypeId,
        element: DecodePlanId,
    },
    Option {
        ty: TypeId,
        element: DecodePlanId,
    },
    Result {
        ty: TypeId,
        error: DecodePlanId,
        value: DecodePlanId,
    },
    Validation {
        ty: TypeId,
        error: DecodePlanId,
        value: DecodePlanId,
    },
}

impl DecodeStep {
    pub fn ty(&self) -> TypeId {
        match *self {
            Self::IntrinsicScalar { ty, .. }
            | Self::Tuple { ty, .. }
            | Self::Record { ty, .. }
            | Self::Sum { ty, .. }
            | Self::Domain { ty, .. }
            | Self::List { ty, .. }
            | Self::Option { ty, .. }
            | Self::Result { ty, .. }
            | Self::Validation { ty, .. } => ty,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeFieldPlan {
    pub name: FieldName,
    pub requirement: DecodeFieldRequirement,
    pub schema: DecodePlanId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeVariantPlan {
    pub name: VariantName,
    pub payload: Option<DecodePlanId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodePlanningError {
    subject: TypeId,
    path: Vec<DecodePathSegment>,
    kind: DecodePlanningErrorKind,
}

impl DecodePlanningError {
    pub fn subject(&self) -> TypeId {
        self.subject
    }

    pub fn path(&self) -> &[DecodePathSegment] {
        &self.path
    }

    pub fn kind(&self) -> &DecodePlanningErrorKind {
        &self.kind
    }
}

impl fmt::Display for DecodePlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            DecodePlanningErrorKind::OpaqueReference { .. } => f.write_str(
                "decode planning reached an opaque leaf that needs later decoder support",
            ),
            DecodePlanningErrorKind::OpenRecord { .. } => {
                f.write_str("decode planning requires records to stay closed")
            }
            DecodePlanningErrorKind::OpenSum { .. } => {
                f.write_str("decode planning requires sums to stay closed")
            }
        }
    }
}

impl Error for DecodePlanningError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodePlanningErrorKind {
    OpaqueReference {
        ty: TypeId,
        reference: TypeReference,
    },
    OpenRecord {
        ty: TypeId,
    },
    OpenSum {
        ty: TypeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodePathSegment {
    TupleElement(usize),
    RecordField(FieldName),
    SumVariantPayload(VariantName),
    DomainCarrier,
    ListElement,
    OptionValue,
    ResultError,
    ResultValue,
    ValidationError,
    ValidationValue,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct DecodePlanner;

impl DecodePlanner {
    // SAFETY NOTE: This planner assumes the type graph is acyclic.
    // If the type store ever supports recursive (mu) types, this function will
    // infinite-loop. When recursive type support is added, a visited-set must
    // be threaded through the traversal to detect and break cycles.
    // See CODE_REVIEW.md §3 (decode.rs problem #9).
    pub fn plan(
        types: &TypeStore,
        subject: TypeId,
        mode: DecodeMode,
    ) -> Result<DecodeSchema, DecodePlanningError> {
        let mut frames = vec![Frame::Enter(subject)];
        let mut path = Vec::new();
        let mut assembled = Vec::new();
        let mut steps = Vec::new();

        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Enter(ty) => match types.node(ty) {
                    TypeNode::Primitive(scalar) => {
                        assembled.push(push_step(
                            &mut steps,
                            DecodeStep::IntrinsicScalar {
                                ty,
                                scalar: *scalar,
                            },
                        ));
                    }
                    TypeNode::Reference(reference) => {
                        return Err(DecodePlanningError {
                            subject,
                            path: path.clone(),
                            kind: DecodePlanningErrorKind::OpaqueReference {
                                ty,
                                reference: *reference,
                            },
                        });
                    }
                    TypeNode::Tuple(elements) => {
                        frames.push(Frame::ExitTuple {
                            ty,
                            arity: elements.len(),
                        });
                        for (index, element) in elements.iter().copied().enumerate().rev() {
                            schedule_child(
                                &mut frames,
                                element,
                                DecodePathSegment::TupleElement(index),
                            );
                        }
                    }
                    TypeNode::Record(record) => {
                        if record.closedness() != Closedness::Closed {
                            return Err(DecodePlanningError {
                                subject,
                                path: path.clone(),
                                kind: DecodePlanningErrorKind::OpenRecord { ty },
                            });
                        }
                        frames.push(Frame::ExitRecord {
                            ty,
                            fields: record
                                .fields()
                                .iter()
                                .map(|field| PendingField {
                                    name: field.name().clone(),
                                    requirement: DecodeFieldRequirement::Required,
                                })
                                .collect(),
                            extra_fields: mode.extra_fields(),
                        });
                        for field in record.fields().iter().rev() {
                            schedule_child(
                                &mut frames,
                                field.ty(),
                                DecodePathSegment::RecordField(field.name().clone()),
                            );
                        }
                    }
                    TypeNode::Sum(sum) => {
                        if sum.closedness() != Closedness::Closed {
                            return Err(DecodePlanningError {
                                subject,
                                path: path.clone(),
                                kind: DecodePlanningErrorKind::OpenSum { ty },
                            });
                        }
                        frames.push(Frame::ExitSum {
                            ty,
                            variants: sum
                                .variants()
                                .iter()
                                .map(|variant| PendingVariant {
                                    name: variant.name().clone(),
                                    has_payload: variant.payload().is_some(),
                                })
                                .collect(),
                            strategy: DecodeSumStrategy::Explicit,
                        });
                        for variant in sum.variants().iter().rev() {
                            if let Some(payload) = variant.payload() {
                                schedule_child(
                                    &mut frames,
                                    payload,
                                    DecodePathSegment::SumVariantPayload(variant.name().clone()),
                                );
                            }
                        }
                    }
                    TypeNode::Domain(domain) => {
                        frames.push(Frame::ExitDomain {
                            ty,
                            rule: DecodeDomainRule::ExplicitSurface,
                        });
                        schedule_child(
                            &mut frames,
                            domain.carrier(),
                            DecodePathSegment::DomainCarrier,
                        );
                    }
                    TypeNode::List(element) => {
                        frames.push(Frame::ExitList { ty });
                        schedule_child(&mut frames, *element, DecodePathSegment::ListElement);
                    }
                    TypeNode::Option(element) => {
                        frames.push(Frame::ExitOption { ty });
                        schedule_child(&mut frames, *element, DecodePathSegment::OptionValue);
                    }
                    TypeNode::Result { error, value } => {
                        frames.push(Frame::ExitResult { ty });
                        schedule_child(&mut frames, *value, DecodePathSegment::ResultValue);
                        schedule_child(&mut frames, *error, DecodePathSegment::ResultError);
                    }
                    TypeNode::Validation { error, value } => {
                        frames.push(Frame::ExitValidation { ty });
                        schedule_child(&mut frames, *value, DecodePathSegment::ValidationValue);
                        schedule_child(&mut frames, *error, DecodePathSegment::ValidationError);
                    }
                },
                Frame::PushPath(segment) => path.push(segment),
                Frame::PopPath => {
                    path.pop().expect("unbalanced decode-planning path frame");
                }
                Frame::ExitTuple { ty, arity } => {
                    let elements = take_tail(&mut assembled, arity);
                    assembled.push(push_step(&mut steps, DecodeStep::Tuple { ty, elements }));
                }
                Frame::ExitRecord {
                    ty,
                    fields,
                    extra_fields,
                } => {
                    let schemas = take_tail(&mut assembled, fields.len());
                    let fields = fields
                        .into_iter()
                        .zip(schemas)
                        .map(|(field, schema)| DecodeFieldPlan {
                            name: field.name,
                            requirement: field.requirement,
                            schema,
                        })
                        .collect();
                    assembled.push(push_step(
                        &mut steps,
                        DecodeStep::Record {
                            ty,
                            fields,
                            extra_fields,
                        },
                    ));
                }
                Frame::ExitSum {
                    ty,
                    variants,
                    strategy,
                } => {
                    let payload_count = variants
                        .iter()
                        .filter(|variant| variant.has_payload)
                        .count();
                    let mut payloads = take_tail(&mut assembled, payload_count).into_iter();
                    let variants = variants
                        .into_iter()
                        .map(|variant| DecodeVariantPlan {
                            name: variant.name,
                            payload: variant.has_payload.then(|| {
                                payloads
                                    .next()
                                    .expect("missing payload schema for decode planning")
                            }),
                        })
                        .collect();
                    assembled.push(push_step(
                        &mut steps,
                        DecodeStep::Sum {
                            ty,
                            variants,
                            strategy,
                        },
                    ));
                }
                Frame::ExitDomain { ty, rule } => {
                    let carrier = pop_one(&mut assembled);
                    assembled.push(push_step(
                        &mut steps,
                        DecodeStep::Domain { ty, carrier, rule },
                    ));
                }
                Frame::ExitList { ty } => {
                    let element = pop_one(&mut assembled);
                    assembled.push(push_step(&mut steps, DecodeStep::List { ty, element }));
                }
                Frame::ExitOption { ty } => {
                    let element = pop_one(&mut assembled);
                    assembled.push(push_step(&mut steps, DecodeStep::Option { ty, element }));
                }
                Frame::ExitResult { ty } => {
                    let mut parts = take_tail(&mut assembled, 2).into_iter();
                    let error = parts.next().expect("missing result error schema");
                    let value = parts.next().expect("missing result value schema");
                    assembled.push(push_step(
                        &mut steps,
                        DecodeStep::Result { ty, error, value },
                    ));
                }
                Frame::ExitValidation { ty } => {
                    let mut parts = take_tail(&mut assembled, 2).into_iter();
                    let error = parts.next().expect("missing validation error schema");
                    let value = parts.next().expect("missing validation value schema");
                    assembled.push(push_step(
                        &mut steps,
                        DecodeStep::Validation { ty, error, value },
                    ));
                }
            }
        }

        let root = pop_one(&mut assembled);
        debug_assert!(assembled.is_empty());

        Ok(DecodeSchema {
            subject,
            mode,
            root,
            steps,
        })
    }
}

#[derive(Clone, Debug)]
enum Frame {
    Enter(TypeId),
    PushPath(DecodePathSegment),
    PopPath,
    ExitTuple {
        ty: TypeId,
        arity: usize,
    },
    ExitRecord {
        ty: TypeId,
        fields: Vec<PendingField>,
        extra_fields: DecodeExtraFieldPolicy,
    },
    ExitSum {
        ty: TypeId,
        variants: Vec<PendingVariant>,
        strategy: DecodeSumStrategy,
    },
    ExitDomain {
        ty: TypeId,
        rule: DecodeDomainRule,
    },
    ExitList {
        ty: TypeId,
    },
    ExitOption {
        ty: TypeId,
    },
    ExitResult {
        ty: TypeId,
    },
    ExitValidation {
        ty: TypeId,
    },
}

#[derive(Clone, Debug)]
struct PendingField {
    name: FieldName,
    requirement: DecodeFieldRequirement,
}

#[derive(Clone, Debug)]
struct PendingVariant {
    name: VariantName,
    has_payload: bool,
}

fn schedule_child(frames: &mut Vec<Frame>, child: TypeId, path: DecodePathSegment) {
    frames.push(Frame::PopPath);
    frames.push(Frame::Enter(child));
    frames.push(Frame::PushPath(path));
}

fn push_step(steps: &mut Vec<DecodeStep>, step: DecodeStep) -> DecodePlanId {
    let id = DecodePlanId::from_index(steps.len());
    steps.push(step);
    id
}

fn take_tail<T>(items: &mut Vec<T>, count: usize) -> Vec<T> {
    let split_at = items
        .len()
        .checked_sub(count)
        .expect("decode planner requested more child schemas than available");
    items.split_off(split_at)
}

fn pop_one<T>(items: &mut Vec<T>) -> T {
    items.pop().expect("missing decode planner child schema")
}

#[cfg(test)]
mod tests {
    use crate::{
        Closedness, DecodeDomainRule, DecodeExtraFieldPolicy, DecodeFieldRequirement, DecodeMode,
        DecodePathSegment, DecodePlanner, DecodePlanningErrorKind, DecodeStep, DecodeSumStrategy,
        PrimitiveType, RecordField, SumVariant, TypeReference, TypeStore,
    };

    #[test]
    fn strict_records_reject_extra_fields_and_keep_all_fields_required() {
        let mut types = TypeStore::new();
        let int = types.primitive(PrimitiveType::Int);
        let text = types.primitive(PrimitiveType::Text);
        let user = types
            .record(
                Closedness::Closed,
                vec![RecordField::new("id", int), RecordField::new("name", text)],
            )
            .expect("closed record should stay valid");

        let schema =
            DecodePlanner::plan(&types, user, DecodeMode::Strict).expect("record should plan");

        match schema.root_step() {
            DecodeStep::Record {
                fields,
                extra_fields,
                ..
            } => {
                assert_eq!(*extra_fields, DecodeExtraFieldPolicy::Reject);
                assert_eq!(fields.len(), 2);
                assert!(fields.iter().all(|field| {
                    field.requirement == DecodeFieldRequirement::Required
                        && matches!(
                            schema.step(field.schema),
                            DecodeStep::IntrinsicScalar {
                                scalar: PrimitiveType::Int | PrimitiveType::Text,
                                ..
                            }
                        )
                }));
            }
            other => panic!("expected record root step, found {other:?}"),
        }
    }

    #[test]
    fn permissive_records_only_relax_extra_field_handling() {
        let mut types = TypeStore::new();
        let int = types.primitive(PrimitiveType::Int);
        let item = types
            .record(Closedness::Closed, vec![RecordField::new("count", int)])
            .expect("closed record should stay valid");

        let schema =
            DecodePlanner::plan(&types, item, DecodeMode::Permissive).expect("record should plan");

        match schema.root_step() {
            DecodeStep::Record {
                fields,
                extra_fields,
                ..
            } => {
                assert_eq!(*extra_fields, DecodeExtraFieldPolicy::Ignore);
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].requirement, DecodeFieldRequirement::Required);
            }
            other => panic!("expected record root step, found {other:?}"),
        }
    }

    #[test]
    fn sums_preserve_explicit_variant_contracts() {
        let mut types = TypeStore::new();
        let text = types.primitive(PrimitiveType::Text);
        let message = types
            .sum(
                Closedness::Closed,
                vec![SumVariant::nullary("Ping"), SumVariant::unary("Data", text)],
            )
            .expect("closed sum should stay valid");

        let schema =
            DecodePlanner::plan(&types, message, DecodeMode::Strict).expect("sum should plan");

        match schema.root_step() {
            DecodeStep::Sum {
                variants, strategy, ..
            } => {
                assert_eq!(*strategy, DecodeSumStrategy::Explicit);
                assert_eq!(variants.len(), 2);
                assert_eq!(variants[0].name.as_str(), "Ping");
                assert_eq!(variants[0].payload, None);
                let payload = variants[1]
                    .payload
                    .expect("payload variant should preserve child schema");
                assert_eq!(variants[1].name.as_str(), "Data");
                assert!(matches!(
                    schema.step(payload),
                    DecodeStep::IntrinsicScalar {
                        scalar: PrimitiveType::Text,
                        ..
                    }
                ));
            }
            other => panic!("expected sum root step, found {other:?}"),
        }
    }

    #[test]
    fn domains_require_explicit_surface_handoff_over_decoded_carriers() {
        let mut types = TypeStore::new();
        let text = types.primitive(PrimitiveType::Text);
        let url = types.domain("Url", text);

        let schema =
            DecodePlanner::plan(&types, url, DecodeMode::Strict).expect("domain should plan");

        match schema.root_step() {
            DecodeStep::Domain { carrier, rule, .. } => {
                assert_eq!(*rule, DecodeDomainRule::ExplicitSurface);
                assert!(matches!(
                    schema.step(*carrier),
                    DecodeStep::IntrinsicScalar {
                        scalar: PrimitiveType::Text,
                        ..
                    }
                ));
            }
            other => panic!("expected domain root step, found {other:?}"),
        }
    }

    #[test]
    fn opaque_references_block_builtin_decode_planning_with_precise_paths() {
        let mut types = TypeStore::new();
        let external = types.define_external("RemoteUser");
        let remote = types.external(external);
        let users = types
            .record(Closedness::Closed, vec![RecordField::new("user", remote)])
            .expect("closed record should stay valid");

        let error = DecodePlanner::plan(&types, users, DecodeMode::Strict)
            .expect_err("opaque references should block builtin decode planning");

        assert_eq!(
            error.kind(),
            &DecodePlanningErrorKind::OpaqueReference {
                ty: remote,
                reference: TypeReference::External(external),
            }
        );
        assert_eq!(
            error.path(),
            &[DecodePathSegment::RecordField("user".into())]
        );
    }

    #[test]
    fn open_shapes_are_rejected_before_runtime_generation() {
        let mut types = TypeStore::new();
        let int = types.primitive(PrimitiveType::Int);
        let open_record = types
            .record(Closedness::Open, vec![RecordField::new("id", int)])
            .expect("open record shape should still build for planner tests");
        let open_sum = types
            .sum(Closedness::Open, vec![SumVariant::nullary("Only")])
            .expect("open sum shape should still build for planner tests");

        let record_error = DecodePlanner::plan(&types, open_record, DecodeMode::Strict)
            .expect_err("open record should be rejected");
        assert_eq!(
            record_error.kind(),
            &DecodePlanningErrorKind::OpenRecord { ty: open_record }
        );

        let sum_error = DecodePlanner::plan(&types, open_sum, DecodeMode::Strict)
            .expect_err("open sum should be rejected");
        assert_eq!(
            sum_error.kind(),
            &DecodePlanningErrorKind::OpenSum { ty: open_sum }
        );
    }
}
