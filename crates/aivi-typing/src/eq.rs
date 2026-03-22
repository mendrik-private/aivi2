use std::collections::BTreeSet;

/// The typeclass family modeled by this crate.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Class {
    Eq,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TypeId(u32);

impl TypeId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("type arena overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TypeParameterId(u32);

impl TypeParameterId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("type parameter table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ExternalTypeId(u32);

impl ExternalTypeId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("external type table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EqPlanId(u32);

impl EqPlanId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("eq plan table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FieldName(Box<str>);

impl FieldName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into().into_boxed_str())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FieldName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VariantName(Box<str>);

impl VariantName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into().into_boxed_str())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for VariantName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Primitive builtins referenced by the structural type model.
///
/// Only a subset currently receives compiler-derived `Eq` witnesses.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PrimitiveType {
    Int,
    Float,
    Decimal,
    BigInt,
    Bool,
    Text,
    Unit,
    Bytes,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Closedness {
    Closed,
    Open,
}

/// A resolved leaf type whose `Eq` evidence must already exist in the surrounding environment.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TypeReference {
    Parameter(TypeParameterId),
    External(ExternalTypeId),
}

/// The small structural type language shared by focused `Eq` and decode planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeNode {
    Primitive(PrimitiveType),
    Reference(TypeReference),
    Tuple(Vec<TypeId>),
    Record(RecordShape),
    Sum(SumShape),
    Domain(DomainShape),
    List(TypeId),
    Option(TypeId),
    Result { error: TypeId, value: TypeId },
    Validation { error: TypeId, value: TypeId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainShape {
    name: Box<str>,
    carrier: TypeId,
}

impl DomainShape {
    pub fn new(name: impl Into<String>, carrier: TypeId) -> Self {
        Self {
            name: name.into().into_boxed_str(),
            carrier,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn carrier(&self) -> TypeId {
        self.carrier
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordShape {
    closedness: Closedness,
    fields: Vec<RecordField>,
}

impl RecordShape {
    pub fn new(closedness: Closedness, fields: Vec<RecordField>) -> Result<Self, ShapeError> {
        validate_unique_field_names(&fields)?;
        Ok(Self { closedness, fields })
    }

    pub fn closedness(&self) -> Closedness {
        self.closedness
    }

    pub fn fields(&self) -> &[RecordField] {
        &self.fields
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordField {
    name: FieldName,
    ty: TypeId,
}

impl RecordField {
    pub fn new(name: impl Into<String>, ty: TypeId) -> Self {
        Self {
            name: FieldName::new(name),
            ty,
        }
    }

    pub fn name(&self) -> &FieldName {
        &self.name
    }

    pub fn ty(&self) -> TypeId {
        self.ty
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SumShape {
    closedness: Closedness,
    variants: Vec<SumVariant>,
}

impl SumShape {
    pub fn new(closedness: Closedness, variants: Vec<SumVariant>) -> Result<Self, ShapeError> {
        validate_unique_variant_names(&variants)?;
        Ok(Self {
            closedness,
            variants,
        })
    }

    pub fn closedness(&self) -> Closedness {
        self.closedness
    }

    pub fn variants(&self) -> &[SumVariant] {
        &self.variants
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SumVariant {
    name: VariantName,
    payload: Option<TypeId>,
}

impl SumVariant {
    pub fn nullary(name: impl Into<String>) -> Self {
        Self {
            name: VariantName::new(name),
            payload: None,
        }
    }

    pub fn unary(name: impl Into<String>, payload: TypeId) -> Self {
        Self {
            name: VariantName::new(name),
            payload: Some(payload),
        }
    }

    pub fn name(&self) -> &VariantName {
        &self.name
    }

    pub fn payload(&self) -> Option<TypeId> {
        self.payload
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapeError {
    kind: ShapeErrorKind,
}

impl ShapeError {
    pub fn kind(&self) -> &ShapeErrorKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShapeErrorKind {
    DuplicateRecordField(FieldName),
    DuplicateSumVariant(VariantName),
    InvalidTupleArity { found: usize },
}

#[derive(Debug, Default)]
pub struct TypeStore {
    nodes: Vec<TypeNode>,
    parameters: Vec<Box<str>>,
    externals: Vec<Box<str>>,
}

impl TypeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define_parameter(&mut self, name: impl Into<String>) -> TypeParameterId {
        let id = TypeParameterId::from_index(self.parameters.len());
        self.parameters.push(name.into().into_boxed_str());
        id
    }

    pub fn define_external(&mut self, name: impl Into<String>) -> ExternalTypeId {
        let id = ExternalTypeId::from_index(self.externals.len());
        self.externals.push(name.into().into_boxed_str());
        id
    }

    pub fn parameter_name(&self, id: TypeParameterId) -> &str {
        &self.parameters[id.index()]
    }

    pub fn external_name(&self, id: ExternalTypeId) -> &str {
        &self.externals[id.index()]
    }

    pub fn primitive(&mut self, primitive: PrimitiveType) -> TypeId {
        self.push(TypeNode::Primitive(primitive))
    }

    pub fn reference(&mut self, reference: TypeReference) -> TypeId {
        self.push(TypeNode::Reference(reference))
    }

    pub fn parameter(&mut self, parameter: TypeParameterId) -> TypeId {
        self.reference(TypeReference::Parameter(parameter))
    }

    pub fn external(&mut self, external: ExternalTypeId) -> TypeId {
        self.reference(TypeReference::External(external))
    }

    pub fn tuple(&mut self, elements: Vec<TypeId>) -> Result<TypeId, ShapeError> {
        if elements.len() < 2 {
            return Err(ShapeError {
                kind: ShapeErrorKind::InvalidTupleArity {
                    found: elements.len(),
                },
            });
        }

        Ok(self.push(TypeNode::Tuple(elements)))
    }

    pub fn record(
        &mut self,
        closedness: Closedness,
        fields: Vec<RecordField>,
    ) -> Result<TypeId, ShapeError> {
        Ok(self.push(TypeNode::Record(RecordShape::new(closedness, fields)?)))
    }

    pub fn sum(
        &mut self,
        closedness: Closedness,
        variants: Vec<SumVariant>,
    ) -> Result<TypeId, ShapeError> {
        Ok(self.push(TypeNode::Sum(SumShape::new(closedness, variants)?)))
    }

    pub fn domain(&mut self, name: impl Into<String>, carrier: TypeId) -> TypeId {
        self.push(TypeNode::Domain(DomainShape::new(name, carrier)))
    }

    pub fn list(&mut self, element: TypeId) -> TypeId {
        self.push(TypeNode::List(element))
    }

    pub fn option(&mut self, element: TypeId) -> TypeId {
        self.push(TypeNode::Option(element))
    }

    pub fn result(&mut self, error: TypeId, value: TypeId) -> TypeId {
        self.push(TypeNode::Result { error, value })
    }

    pub fn validation(&mut self, error: TypeId, value: TypeId) -> TypeId {
        self.push(TypeNode::Validation { error, value })
    }

    pub fn node(&self, id: TypeId) -> &TypeNode {
        &self.nodes[id.index()]
    }

    fn push(&mut self, node: TypeNode) -> TypeId {
        let id = TypeId::from_index(self.nodes.len());
        self.nodes.push(node);
        id
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EqContext {
    assumptions: BTreeSet<TypeReference>,
}

impl EqContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn assume(&mut self, reference: TypeReference) -> bool {
        self.assumptions.insert(reference)
    }

    pub fn assume_parameter(&mut self, parameter: TypeParameterId) -> bool {
        self.assume(TypeReference::Parameter(parameter))
    }

    pub fn assume_external(&mut self, external: ExternalTypeId) -> bool {
        self.assume(TypeReference::External(external))
    }

    pub fn contains(&self, reference: TypeReference) -> bool {
        self.assumptions.contains(&reference)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct InstanceHead {
    pub class: Class,
    pub subject: TypeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EqDerivation {
    head: InstanceHead,
    root: EqPlanId,
    steps: Vec<EqStep>,
}

impl EqDerivation {
    pub fn head(&self) -> InstanceHead {
        self.head
    }

    pub fn root(&self) -> EqPlanId {
        self.root
    }

    pub fn root_step(&self) -> &EqStep {
        self.step(self.root)
    }

    pub fn step(&self, id: EqPlanId) -> &EqStep {
        &self.steps[id.index()]
    }

    pub fn steps(&self) -> &[EqStep] {
        &self.steps
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqStep {
    IntrinsicScalar {
        ty: TypeId,
        scalar: PrimitiveType,
    },
    FromContext {
        ty: TypeId,
        reference: TypeReference,
    },
    Tuple {
        ty: TypeId,
        elements: Vec<EqPlanId>,
    },
    Record {
        ty: TypeId,
        fields: Vec<EqFieldPlan>,
    },
    Sum {
        ty: TypeId,
        variants: Vec<EqVariantPlan>,
    },
    Domain {
        ty: TypeId,
        carrier: EqPlanId,
    },
    List {
        ty: TypeId,
        element: EqPlanId,
    },
    Option {
        ty: TypeId,
        element: EqPlanId,
    },
    Result {
        ty: TypeId,
        error: EqPlanId,
        value: EqPlanId,
    },
    Validation {
        ty: TypeId,
        error: EqPlanId,
        value: EqPlanId,
    },
}

impl EqStep {
    pub fn ty(&self) -> TypeId {
        match *self {
            Self::IntrinsicScalar { ty, .. }
            | Self::FromContext { ty, .. }
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
pub struct EqFieldPlan {
    pub name: FieldName,
    pub witness: EqPlanId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EqVariantPlan {
    pub name: VariantName,
    pub payload: Option<EqPlanId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EqDerivationError {
    head: InstanceHead,
    path: Vec<EqPathSegment>,
    kind: EqDerivationErrorKind,
}

impl EqDerivationError {
    pub fn head(&self) -> InstanceHead {
        self.head
    }

    pub fn path(&self) -> &[EqPathSegment] {
        &self.path
    }

    pub fn kind(&self) -> &EqDerivationErrorKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqDerivationErrorKind {
    MissingEq {
        ty: TypeId,
        reference: TypeReference,
    },
    UnsupportedPrimitive {
        ty: TypeId,
        primitive: PrimitiveType,
    },
    OpenRecord {
        ty: TypeId,
    },
    OpenSum {
        ty: TypeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqPathSegment {
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
pub struct EqDeriver;

impl EqDeriver {
    pub fn derive(
        types: &TypeStore,
        subject: TypeId,
        context: &EqContext,
    ) -> Result<EqDerivation, EqDerivationError> {
        let head = InstanceHead {
            class: Class::Eq,
            subject,
        };
        let mut frames = vec![Frame::Enter(subject)];
        let mut path = Vec::new();
        let mut assembled = Vec::new();
        let mut steps = Vec::new();

        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Enter(ty) => match types.node(ty) {
                    TypeNode::Primitive(scalar) => match scalar {
                        PrimitiveType::Bytes => {
                            return Err(EqDerivationError {
                                head,
                                path: path.clone(),
                                kind: EqDerivationErrorKind::UnsupportedPrimitive {
                                    ty,
                                    primitive: *scalar,
                                },
                            });
                        }
                        _ => {
                            assembled.push(push_step(
                                &mut steps,
                                EqStep::IntrinsicScalar {
                                    ty,
                                    scalar: *scalar,
                                },
                            ));
                        }
                    },
                    TypeNode::Reference(reference) => {
                        if context.contains(*reference) {
                            assembled.push(push_step(
                                &mut steps,
                                EqStep::FromContext {
                                    ty,
                                    reference: *reference,
                                },
                            ));
                        } else {
                            return Err(EqDerivationError {
                                head,
                                path: path.clone(),
                                kind: EqDerivationErrorKind::MissingEq {
                                    ty,
                                    reference: *reference,
                                },
                            });
                        }
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
                                EqPathSegment::TupleElement(index),
                            );
                        }
                    }
                    TypeNode::Record(record) => {
                        if record.closedness() != Closedness::Closed {
                            return Err(EqDerivationError {
                                head,
                                path: path.clone(),
                                kind: EqDerivationErrorKind::OpenRecord { ty },
                            });
                        }

                        frames.push(Frame::ExitRecord {
                            ty,
                            field_names: record
                                .fields()
                                .iter()
                                .map(|field| field.name().clone())
                                .collect(),
                        });
                        for field in record.fields().iter().rev() {
                            schedule_child(
                                &mut frames,
                                field.ty(),
                                EqPathSegment::RecordField(field.name().clone()),
                            );
                        }
                    }
                    TypeNode::Sum(sum) => {
                        if sum.closedness() != Closedness::Closed {
                            return Err(EqDerivationError {
                                head,
                                path: path.clone(),
                                kind: EqDerivationErrorKind::OpenSum { ty },
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
                        });
                        for variant in sum.variants().iter().rev() {
                            if let Some(payload) = variant.payload() {
                                schedule_child(
                                    &mut frames,
                                    payload,
                                    EqPathSegment::SumVariantPayload(variant.name().clone()),
                                );
                            }
                        }
                    }
                    TypeNode::Domain(domain) => {
                        frames.push(Frame::ExitDomain { ty });
                        schedule_child(&mut frames, domain.carrier(), EqPathSegment::DomainCarrier);
                    }
                    TypeNode::List(element) => {
                        frames.push(Frame::ExitList { ty });
                        schedule_child(&mut frames, *element, EqPathSegment::ListElement);
                    }
                    TypeNode::Option(element) => {
                        frames.push(Frame::ExitOption { ty });
                        schedule_child(&mut frames, *element, EqPathSegment::OptionValue);
                    }
                    TypeNode::Result { error, value } => {
                        frames.push(Frame::ExitResult { ty });
                        schedule_child(&mut frames, *value, EqPathSegment::ResultValue);
                        schedule_child(&mut frames, *error, EqPathSegment::ResultError);
                    }
                    TypeNode::Validation { error, value } => {
                        frames.push(Frame::ExitValidation { ty });
                        schedule_child(&mut frames, *value, EqPathSegment::ValidationValue);
                        schedule_child(&mut frames, *error, EqPathSegment::ValidationError);
                    }
                },
                Frame::PushPath(segment) => path.push(segment),
                Frame::PopPath => {
                    path.pop().expect("unbalanced derivation path frame");
                }
                Frame::ExitTuple { ty, arity } => {
                    let elements = take_tail(&mut assembled, arity);
                    assembled.push(push_step(&mut steps, EqStep::Tuple { ty, elements }));
                }
                Frame::ExitRecord { ty, field_names } => {
                    let witnesses = take_tail(&mut assembled, field_names.len());
                    let fields = field_names
                        .into_iter()
                        .zip(witnesses)
                        .map(|(name, witness)| EqFieldPlan { name, witness })
                        .collect();
                    assembled.push(push_step(&mut steps, EqStep::Record { ty, fields }));
                }
                Frame::ExitSum { ty, variants } => {
                    let payload_count = variants
                        .iter()
                        .filter(|variant| variant.has_payload)
                        .count();
                    let mut payloads = take_tail(&mut assembled, payload_count).into_iter();
                    let variants = variants
                        .into_iter()
                        .map(|variant| EqVariantPlan {
                            name: variant.name,
                            payload: variant.has_payload.then(|| {
                                payloads
                                    .next()
                                    .expect("missing payload witness for sum derivation")
                            }),
                        })
                        .collect();
                    assembled.push(push_step(&mut steps, EqStep::Sum { ty, variants }));
                }
                Frame::ExitDomain { ty } => {
                    let carrier = pop_one(&mut assembled);
                    assembled.push(push_step(&mut steps, EqStep::Domain { ty, carrier }));
                }
                Frame::ExitList { ty } => {
                    let element = pop_one(&mut assembled);
                    assembled.push(push_step(&mut steps, EqStep::List { ty, element }));
                }
                Frame::ExitOption { ty } => {
                    let element = pop_one(&mut assembled);
                    assembled.push(push_step(&mut steps, EqStep::Option { ty, element }));
                }
                Frame::ExitResult { ty } => {
                    let mut parts = take_tail(&mut assembled, 2).into_iter();
                    let error = parts.next().expect("missing result error witness");
                    let value = parts.next().expect("missing result value witness");
                    assembled.push(push_step(&mut steps, EqStep::Result { ty, error, value }));
                }
                Frame::ExitValidation { ty } => {
                    let mut parts = take_tail(&mut assembled, 2).into_iter();
                    let error = parts.next().expect("missing validation error witness");
                    let value = parts.next().expect("missing validation value witness");
                    assembled.push(push_step(
                        &mut steps,
                        EqStep::Validation { ty, error, value },
                    ));
                }
            }
        }

        let root = pop_one(&mut assembled);
        debug_assert!(assembled.is_empty());

        Ok(EqDerivation { head, root, steps })
    }
}

#[derive(Clone, Debug)]
enum Frame {
    Enter(TypeId),
    PushPath(EqPathSegment),
    PopPath,
    ExitTuple {
        ty: TypeId,
        arity: usize,
    },
    ExitRecord {
        ty: TypeId,
        field_names: Vec<FieldName>,
    },
    ExitSum {
        ty: TypeId,
        variants: Vec<PendingVariant>,
    },
    ExitDomain {
        ty: TypeId,
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
struct PendingVariant {
    name: VariantName,
    has_payload: bool,
}

fn schedule_child(frames: &mut Vec<Frame>, child: TypeId, path: EqPathSegment) {
    frames.push(Frame::PopPath);
    frames.push(Frame::Enter(child));
    frames.push(Frame::PushPath(path));
}

fn push_step(steps: &mut Vec<EqStep>, step: EqStep) -> EqPlanId {
    let id = EqPlanId::from_index(steps.len());
    steps.push(step);
    id
}

fn take_tail<T>(items: &mut Vec<T>, count: usize) -> Vec<T> {
    let split_at = items
        .len()
        .checked_sub(count)
        .expect("derivation frame requested more witnesses than available");
    items.split_off(split_at)
}

fn pop_one<T>(items: &mut Vec<T>) -> T {
    items.pop().expect("missing derivation witness")
}

fn validate_unique_field_names(fields: &[RecordField]) -> Result<(), ShapeError> {
    let mut seen = BTreeSet::new();
    for field in fields {
        if !seen.insert(field.name().clone()) {
            return Err(ShapeError {
                kind: ShapeErrorKind::DuplicateRecordField(field.name().clone()),
            });
        }
    }
    Ok(())
}

fn validate_unique_variant_names(variants: &[SumVariant]) -> Result<(), ShapeError> {
    let mut seen = BTreeSet::new();
    for variant in variants {
        if !seen.insert(variant.name().clone()) {
            return Err(ShapeError {
                kind: ShapeErrorKind::DuplicateSumVariant(variant.name().clone()),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_scalars_use_intrinsic_eq_witnesses() {
        let mut types = TypeStore::new();
        let context = EqContext::new();
        let primitives = [
            PrimitiveType::Int,
            PrimitiveType::Float,
            PrimitiveType::Decimal,
            PrimitiveType::BigInt,
            PrimitiveType::Bool,
            PrimitiveType::Text,
            PrimitiveType::Unit,
        ];

        for primitive in primitives {
            let ty = types.primitive(primitive);
            let derivation =
                EqDeriver::derive(&types, ty, &context).expect("primitive should derive");

            assert_eq!(
                derivation.head(),
                InstanceHead {
                    class: Class::Eq,
                    subject: ty,
                }
            );
            assert!(
                matches!(
                    derivation.root_step(),
                    EqStep::IntrinsicScalar { ty: root_ty, scalar }
                        if *root_ty == ty && *scalar == primitive
                ),
                "expected intrinsic eq for {primitive:?}"
            );
        }
    }

    #[test]
    fn standard_wrappers_reuse_context_evidence_for_generic_members() {
        let mut types = TypeStore::new();
        let left = types.define_parameter("E");
        let right = types.define_parameter("A");
        let left_ty = types.parameter(left);
        let right_ty = types.parameter(right);
        let list_ty = types.list(right_ty);
        let option_ty = types.option(left_ty);
        let result_ty = types.result(left_ty, right_ty);
        let validation_ty = types.validation(left_ty, right_ty);
        let subject = types
            .tuple(vec![list_ty, option_ty, result_ty, validation_ty])
            .expect("tuple arity is valid");

        let mut context = EqContext::new();
        context.assume_parameter(left);
        context.assume_parameter(right);

        let derivation = EqDeriver::derive(&types, subject, &context)
            .expect("generic wrapper tuple should derive");
        let EqStep::Tuple { elements, .. } = derivation.root_step() else {
            panic!("expected tuple root");
        };
        assert_eq!(elements.len(), 4);

        let EqStep::List { element, .. } = derivation.step(elements[0]) else {
            panic!("expected list witness");
        };
        assert!(matches!(
            derivation.step(*element),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == right
        ));

        let EqStep::Option { element, .. } = derivation.step(elements[1]) else {
            panic!("expected option witness");
        };
        assert!(matches!(
            derivation.step(*element),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == left
        ));

        let EqStep::Result { error, value, .. } = derivation.step(elements[2]) else {
            panic!("expected result witness");
        };
        assert!(matches!(
            derivation.step(*error),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == left
        ));
        assert!(matches!(
            derivation.step(*value),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == right
        ));

        let EqStep::Validation { error, value, .. } = derivation.step(elements[3]) else {
            panic!("expected validation witness");
        };
        assert!(matches!(
            derivation.step(*error),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == left
        ));
        assert!(matches!(
            derivation.step(*value),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == right
        ));
    }

    #[test]
    fn domains_keep_nominal_roots_but_reuse_carrier_eq() {
        let mut types = TypeStore::new();
        let int_ty = types.primitive(PrimitiveType::Int);
        let duration = types.domain("Duration", int_ty);

        let derivation =
            EqDeriver::derive(&types, duration, &EqContext::new()).expect("domain should derive");
        let EqStep::Domain { ty, carrier } = derivation.root_step() else {
            panic!("expected domain root witness");
        };
        assert_eq!(*ty, duration);
        assert!(matches!(
            derivation.step(*carrier),
            EqStep::IntrinsicScalar {
                ty: carrier_ty,
                scalar: PrimitiveType::Int,
            } if *carrier_ty == int_ty
        ));

        let parameter = types.define_parameter("A");
        let element = types.parameter(parameter);
        let carrier = types.list(element);
        let non_empty = types.domain("NonEmpty", carrier);
        let mut context = EqContext::new();
        context.assume_parameter(parameter);

        let derivation =
            EqDeriver::derive(&types, non_empty, &context).expect("generic domain should derive");
        let EqStep::Domain { carrier, .. } = derivation.root_step() else {
            panic!("expected generic domain root witness");
        };
        let EqStep::List { element, .. } = derivation.step(*carrier) else {
            panic!("expected carrier list witness");
        };
        assert!(matches!(
            derivation.step(*element),
            EqStep::FromContext {
                reference: TypeReference::Parameter(id),
                ..
            } if *id == parameter
        ));
    }

    #[test]
    fn bytes_is_explicitly_not_compiler_derived_in_v1() {
        let mut types = TypeStore::new();
        let bytes = types.primitive(PrimitiveType::Bytes);

        let error = EqDeriver::derive(&types, bytes, &EqContext::new())
            .expect_err("bytes should not derive");
        assert_eq!(
            error.kind(),
            &EqDerivationErrorKind::UnsupportedPrimitive {
                ty: bytes,
                primitive: PrimitiveType::Bytes,
            }
        );
        assert!(error.path().is_empty());
    }

    #[test]
    fn closed_records_preserve_field_order_and_require_member_eq() {
        let mut types = TypeStore::new();
        let user_id = types.define_external("UserId");
        let user_id_ty = types.external(user_id);
        let text_ty = types.primitive(PrimitiveType::Text);
        let tag_ty = types.list(text_ty);
        let bool_ty = types.primitive(PrimitiveType::Bool);
        let subject = types
            .record(
                Closedness::Closed,
                vec![
                    RecordField::new("id", user_id_ty),
                    RecordField::new("tags", tag_ty),
                    RecordField::new("active", bool_ty),
                ],
            )
            .expect("closed record shape is valid");

        let mut context = EqContext::new();
        context.assume_external(user_id);

        let derivation =
            EqDeriver::derive(&types, subject, &context).expect("record should derive");
        let EqStep::Record { fields, .. } = derivation.root_step() else {
            panic!("expected record witness");
        };
        assert_eq!(
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "tags", "active"]
        );
        assert!(matches!(
            derivation.step(fields[0].witness),
            EqStep::FromContext {
                reference: TypeReference::External(id),
                ..
            } if *id == user_id
        ));
        let EqStep::List { element, .. } = derivation.step(fields[1].witness) else {
            panic!("expected list field witness");
        };
        assert!(matches!(
            derivation.step(*element),
            EqStep::IntrinsicScalar {
                scalar: PrimitiveType::Text,
                ..
            }
        ));
        assert!(matches!(
            derivation.step(fields[2].witness),
            EqStep::IntrinsicScalar {
                scalar: PrimitiveType::Bool,
                ..
            }
        ));
    }

    #[test]
    fn closed_sums_keep_nullary_variants_and_payload_witnesses() {
        let mut types = TypeStore::new();
        let text_ty = types.primitive(PrimitiveType::Text);
        let payload = types.option(text_ty);
        let subject = types
            .sum(
                Closedness::Closed,
                vec![
                    SumVariant::nullary("NoValue"),
                    SumVariant::unary("HasValue", payload),
                ],
            )
            .expect("closed sum shape is valid");

        let derivation = EqDeriver::derive(&types, subject, &EqContext::new())
            .expect("closed sum should derive");
        let EqStep::Sum { variants, .. } = derivation.root_step() else {
            panic!("expected sum witness");
        };
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].name.as_str(), "NoValue");
        assert!(variants[0].payload.is_none());
        assert_eq!(variants[1].name.as_str(), "HasValue");

        let payload_witness = variants[1]
            .payload
            .expect("payload variant should retain witness");
        let EqStep::Option { element, .. } = derivation.step(payload_witness) else {
            panic!("expected option witness");
        };
        assert!(matches!(
            derivation.step(*element),
            EqStep::IntrinsicScalar {
                scalar: PrimitiveType::Text,
                ..
            }
        ));
    }

    #[test]
    fn open_records_and_sums_are_rejected() {
        let mut types = TypeStore::new();
        let scalar = types.primitive(PrimitiveType::Text);
        let open_record = types
            .record(Closedness::Open, vec![RecordField::new("name", scalar)])
            .expect("shape is syntactically valid");
        let open_sum = types
            .sum(Closedness::Open, vec![SumVariant::unary("Named", scalar)])
            .expect("shape is syntactically valid");

        let record_error = EqDeriver::derive(&types, open_record, &EqContext::new())
            .expect_err("open records cannot derive");
        assert_eq!(
            record_error.kind(),
            &EqDerivationErrorKind::OpenRecord { ty: open_record }
        );
        assert!(record_error.path().is_empty());

        let sum_error = EqDeriver::derive(&types, open_sum, &EqContext::new())
            .expect_err("open sums cannot derive");
        assert_eq!(
            sum_error.kind(),
            &EqDerivationErrorKind::OpenSum { ty: open_sum }
        );
        assert!(sum_error.path().is_empty());
    }

    #[test]
    fn missing_eq_reports_the_nested_path() {
        let mut types = TypeStore::new();
        let missing = types.define_external("MissingEq");
        let text_ty = types.primitive(PrimitiveType::Text);
        let missing_ty = types.external(missing);
        let blocked_sum = types
            .sum(
                Closedness::Closed,
                vec![
                    SumVariant::nullary("Ready"),
                    SumVariant::unary("Blocked", missing_ty),
                ],
            )
            .expect("sum is valid");
        let subject = types.result(text_ty, blocked_sum);

        let error = EqDeriver::derive(&types, subject, &EqContext::new())
            .expect_err("missing external eq should fail");
        assert_eq!(
            error.kind(),
            &EqDerivationErrorKind::MissingEq {
                ty: missing_ty,
                reference: TypeReference::External(missing),
            }
        );
        assert_eq!(
            error.path(),
            &[
                EqPathSegment::ResultValue,
                EqPathSegment::SumVariantPayload(VariantName::new("Blocked")),
            ]
        );
    }

    #[test]
    fn domain_missing_eq_reports_the_carrier_path() {
        let mut types = TypeStore::new();
        let missing = types.define_external("MissingEq");
        let missing_ty = types.external(missing);
        let wrapped = types.option(missing_ty);
        let subject = types.domain("Secret", wrapped);

        let error = EqDeriver::derive(&types, subject, &EqContext::new())
            .expect_err("domain carrier without eq should fail");
        assert_eq!(
            error.kind(),
            &EqDerivationErrorKind::MissingEq {
                ty: missing_ty,
                reference: TypeReference::External(missing),
            }
        );
        assert_eq!(
            error.path(),
            &[EqPathSegment::DomainCarrier, EqPathSegment::OptionValue]
        );
    }

    #[test]
    fn duplicate_names_and_invalid_tuple_arity_are_rejected() {
        let mut types = TypeStore::new();
        let scalar = types.primitive(PrimitiveType::Int);

        let record_error = types
            .record(
                Closedness::Closed,
                vec![
                    RecordField::new("id", scalar),
                    RecordField::new("id", scalar),
                ],
            )
            .expect_err("duplicate record fields should fail");
        assert_eq!(
            record_error.kind(),
            &ShapeErrorKind::DuplicateRecordField(FieldName::new("id"))
        );

        let sum_error = types
            .sum(
                Closedness::Closed,
                vec![SumVariant::nullary("One"), SumVariant::nullary("One")],
            )
            .expect_err("duplicate sum variants should fail");
        assert_eq!(
            sum_error.kind(),
            &ShapeErrorKind::DuplicateSumVariant(VariantName::new("One"))
        );

        let tuple_error = types
            .tuple(vec![scalar])
            .expect_err("single-element tuples are intentionally unsupported");
        assert_eq!(
            tuple_error.kind(),
            &ShapeErrorKind::InvalidTupleArity { found: 1 }
        );
    }

    #[test]
    fn deep_option_nesting_is_derived_without_recursive_walkers() {
        let mut types = TypeStore::new();
        let atom = types.define_external("Atom");
        let mut current = types.external(atom);
        for _ in 0..2_048 {
            current = types.option(current);
        }

        let mut context = EqContext::new();
        context.assume_external(atom);

        let derivation = EqDeriver::derive(&types, current, &context)
            .expect("deep option nesting should derive");
        assert_eq!(derivation.steps().len(), 2_049);
        assert!(matches!(derivation.root_step(), EqStep::Option { .. }));
    }
}
