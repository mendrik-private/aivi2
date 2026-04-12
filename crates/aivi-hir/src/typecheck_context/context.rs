pub(crate) struct GateTypeContext<'a> {
    module: &'a Module,
    item_types: HashMap<ItemId, Option<GateType>>,
    item_actuals: HashMap<ItemId, Option<SourceOptionActualType>>,
    inferred_function_types: Option<HashMap<ItemId, GateType>>,
    function_call_evidence: Vec<FunctionCallEvidence>,
    function_signature_evidence: Vec<FunctionSignatureEvidence>,
    allow_function_inference: bool,
}

impl<'a> GateTypeContext<'a> {
    pub(crate) fn new(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
            item_actuals: HashMap::new(),
            inferred_function_types: None,
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference: true,
        }
    }

    pub(crate) fn new_for_function_inference(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
            item_actuals: HashMap::new(),
            inferred_function_types: Some(HashMap::new()),
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference: false,
        }
    }

    pub(crate) fn with_seeded_item_types(
        module: &'a Module,
        item_types: HashMap<ItemId, GateType>,
        allow_function_inference: bool,
    ) -> Self {
        Self {
            module,
            item_types: item_types
                .into_iter()
                .map(|(item_id, ty)| (item_id, Some(ty)))
                .collect(),
            item_actuals: HashMap::new(),
            inferred_function_types: Some(HashMap::new()),
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference,
        }
    }

    fn inferred_function_types(&mut self) -> &HashMap<ItemId, GateType> {
        self.inferred_function_types
            .get_or_insert_with(|| infer_same_module_function_types(self.module))
    }

    pub(crate) fn record_function_call_evidence(&mut self, evidence: FunctionCallEvidence) {
        self.function_call_evidence.push(evidence);
    }

    pub(crate) fn take_function_call_evidence(&mut self) -> Vec<FunctionCallEvidence> {
        std::mem::take(&mut self.function_call_evidence)
    }

    pub(crate) fn record_function_signature_evidence(
        &mut self,
        evidence: FunctionSignatureEvidence,
    ) {
        self.function_signature_evidence.push(evidence);
    }

    pub(crate) fn take_function_signature_evidence(&mut self) -> Vec<FunctionSignatureEvidence> {
        std::mem::take(&mut self.function_signature_evidence)
    }

    pub(crate) fn fanout_carrier(&self, subject: &GateType) -> Option<FanoutCarrier> {
        subject.fanout_carrier()
    }

    pub(crate) fn gate_carrier(&self, subject: &GateType) -> GateCarrier {
        subject.gate_carrier()
    }

    pub(crate) fn truthy_falsy_subject_plan(
        &self,
        subject: &GateType,
    ) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Signal(inner) => Self::truthy_falsy_ordinary_subject_plan(inner),
            other => Self::truthy_falsy_ordinary_subject_plan(other),
        }
    }

    fn arrow_parameter_types(ty: &GateType, arity: usize) -> Option<Vec<GateType>> {
        let mut current = ty;
        let mut parameter_types = Vec::with_capacity(arity);
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameter_types.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some(parameter_types)
    }

    fn arrow_result_type(ty: &GateType, arity: usize) -> Option<GateType> {
        let mut current = ty;
        for _ in 0..arity {
            let GateType::Arrow { result, .. } = current else {
                return None;
            };
            current = result.as_ref();
        }
        Some(current.clone())
    }

    pub(crate) fn truthy_falsy_ordinary_subject_plan(
        subject: &GateType,
    ) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::True,
                truthy_payload: None,
                falsy_constructor: BuiltinTerm::False,
                falsy_payload: None,
            }),
            GateType::Option(payload) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Some,
                truthy_payload: Some(payload.as_ref().clone()),
                falsy_constructor: BuiltinTerm::None,
                falsy_payload: None,
            }),
            GateType::Result { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Ok,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Err,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Validation { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Valid,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Invalid,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. }
            | GateType::OpaqueItem { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn apply_truthy_falsy_result_type(
        &self,
        subject: &GateType,
        result: GateType,
    ) -> GateType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => GateType::Signal(Box::new(result)),
        }
    }

    pub(crate) fn apply_truthy_falsy_result_actual(
        &self,
        subject: &GateType,
        result: SourceOptionActualType,
    ) -> SourceOptionActualType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(result)),
        }
    }

    pub(crate) fn case_subject_shape(&mut self, subject: &GateType) -> Option<CaseSubjectShape> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::True),
                        display: "True".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::False),
                        display: "False".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Option(payload) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Some),
                        display: "Some".to_owned(),
                        span: None,
                        field_types: Some(vec![payload.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::None),
                        display: "None".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Result { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Ok),
                        display: "Ok".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Err),
                        display: "Err".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::Validation { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Valid),
                        display: "Valid".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Invalid),
                        display: "Invalid".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::OpaqueItem {
                item, arguments, ..
            } => self.same_module_case_subject_shape(*item, arguments),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn same_module_case_subject_shape(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
    ) -> Option<CaseSubjectShape> {
        let Item::Type(item) = &self.module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        if item.parameters.len() != arguments.len() {
            return None;
        }
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let constructors = variants
            .iter()
            .map(|variant| CaseConstructorShape {
                key: CaseConstructorKey::SameModuleVariant {
                    item: item_id,
                    name: variant.name.text().to_owned(),
                },
                display: variant.name.text().to_owned(),
                span: Some(variant.span),
                field_types: self.lower_case_variant_fields(
                    &variant.fields.iter().map(|f| f.ty).collect::<Vec<_>>(),
                    &substitutions,
                ),
            })
            .collect::<Vec<_>>();
        Some(CaseSubjectShape { constructors })
    }

    pub(crate) fn lower_case_variant_fields(
        &mut self,
        fields: &[TypeId],
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<Vec<GateType>> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            lowered.push(self.lower_type(*field, substitutions, &mut Vec::new(), false)?);
        }
        Some(lowered)
    }

    pub(crate) fn case_pattern_coverage(
        &mut self,
        pattern_id: PatternId,
        subject: &CaseSubjectShape,
    ) -> CasePatternCoverage {
        let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
            return CasePatternCoverage::None;
        };
        match pattern.kind {
            PatternKind::Wildcard | PatternKind::Binding(_) => CasePatternCoverage::CatchAll,
            PatternKind::Constructor { callee, .. } | PatternKind::UnresolvedName(callee) => {
                let Some(key) = case_constructor_key(&callee) else {
                    return CasePatternCoverage::None;
                };
                if subject.constructor(&key).is_some() {
                    CasePatternCoverage::Constructor(key)
                } else {
                    CasePatternCoverage::None
                }
            }
            PatternKind::Integer(_)
            | PatternKind::Text(_)
            | PatternKind::Tuple(_)
            | PatternKind::List { .. }
            | PatternKind::Record(_) => CasePatternCoverage::None,
        }
    }

    pub(crate) fn case_pattern_bindings(
        &mut self,
        pattern_id: PatternId,
        subject: &GateType,
    ) -> GateExprEnv {
        let mut env = GateExprEnv::default();
        let mut work = vec![(pattern_id, subject.clone())];
        while let Some((pattern_id, subject_ty)) = work.pop() {
            let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
                continue;
            };
            match pattern.kind {
                PatternKind::Wildcard
                | PatternKind::Integer(_)
                | PatternKind::Text(_)
                | PatternKind::UnresolvedName(_) => {}
                PatternKind::Binding(binding) => {
                    env.locals.insert(binding.binding, subject_ty);
                }
                PatternKind::Tuple(elements) => {
                    let GateType::Tuple(subject_elements) = &subject_ty else {
                        continue;
                    };
                    if elements.len() != subject_elements.len() {
                        continue;
                    }
                    let element_pairs = elements
                        .iter()
                        .zip(subject_elements.iter())
                        .collect::<Vec<_>>();
                    for (element, element_ty) in element_pairs.into_iter().rev() {
                        work.push((*element, element_ty.clone()));
                    }
                }
                PatternKind::List { elements, rest } => {
                    let GateType::List(element_ty) = &subject_ty else {
                        continue;
                    };
                    for element in elements.into_iter().rev() {
                        work.push((element, element_ty.as_ref().clone()));
                    }
                    if let Some(rest) = rest {
                        work.push((rest, subject_ty));
                    }
                }
                PatternKind::Record(fields) => {
                    // Collect (name, type) pairs from either a local record or an imported one.
                    let record_pairs: Option<Vec<(String, GateType)>> = match &subject_ty {
                        GateType::Record(subject_fields) => Some(
                            subject_fields
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                        ),
                        GateType::OpaqueImport { import, .. } => {
                            match self.module.imports().get(*import).map(|b| &b.metadata) {
                                Some(ImportBindingMetadata::TypeConstructor {
                                    fields: Some(import_fields),
                                    ..
                                }) => Some(
                                    import_fields
                                        .iter()
                                        .map(|f| {
                                            (
                                                f.name.to_string(),
                                                self.lower_import_value_type(&f.ty),
                                            )
                                        })
                                        .collect(),
                                ),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    let Some(record_pairs) = record_pairs else {
                        continue;
                    };
                    for field in fields.into_iter().rev() {
                        let Some(field_ty) = record_pairs
                            .iter()
                            .find(|(name, _)| name.as_str() == field.label.text())
                            .map(|(_, ty)| ty.clone())
                        else {
                            continue;
                        };
                        work.push((field.pattern, field_ty));
                    }
                }
                PatternKind::Constructor { callee, arguments } => {
                    let Some(field_types) = self.case_pattern_field_types(&callee, &subject_ty)
                    else {
                        continue;
                    };
                    if field_types.len() != arguments.len() {
                        continue;
                    }
                    for (argument, field_ty) in arguments.into_iter().zip(field_types).rev() {
                        work.push((argument, field_ty));
                    }
                }
            }
        }
        env
    }

    pub(crate) fn case_pattern_field_types(
        &mut self,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let key = case_constructor_key(callee)?;
        // For imported constructors, extract field types from the Arrow chain in the import type.
        if let CaseConstructorKey::ImportedVariant { import, .. } = &key {
            return Some(self.import_constructor_field_types(*import));
        }
        let subject = self.case_subject_shape(subject)?;
        subject.constructor(&key)?.field_types.clone()
    }

    fn import_constructor_field_types(&mut self, import_id: ImportId) -> Vec<GateType> {
        let ty = {
            let binding = &self.module.imports()[import_id];
            match &binding.metadata {
                crate::ImportBindingMetadata::Value { ty } => ty.clone(),
                _ => return Vec::new(),
            }
        };
        // Collect all Arrow `parameter` types from the chain; the final `result` is the sum type.
        let mut fields = Vec::new();
        let mut current = &ty;
        while let ImportValueType::Arrow { parameter, result } = current {
            fields.push(self.lower_import_value_type(parameter));
            current = result;
        }
        fields
    }

    pub(crate) fn apply_fanout_plan(&self, plan: FanoutPlan, subject: GateType) -> GateType {
        match plan.result() {
            FanoutResultKind::MappedCollection => {
                let mapped_collection = GateType::List(Box::new(subject));
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(mapped_collection))
                } else {
                    mapped_collection
                }
            }
            FanoutResultKind::JoinedValue => {
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(subject))
                } else {
                    subject
                }
            }
        }
    }

    pub(crate) fn apply_gate_plan(
        &self,
        plan: aivi_typing::GatePlan,
        subject: &GateType,
    ) -> GateType {
        match plan.result() {
            GateResultKind::OptionWrappedSubject => GateType::Option(Box::new(subject.clone())),
            GateResultKind::PreservedSignalSubject => match subject {
                GateType::Signal(_) => subject.clone(),
                other => GateType::Signal(Box::new(other.clone())),
            },
        }
    }

    pub(crate) fn lower_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), false)
    }

    pub(crate) fn lower_open_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), true)
    }

    pub(crate) fn lower_hir_type(
        &mut self,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        self.lower_type(ty, substitutions, &mut Vec::new(), false)
    }

    pub(crate) fn poly_type_binding(&mut self, ty: TypeId) -> Option<TypeBinding> {
        if let Some(lowered) = self.lower_annotation(ty) {
            return Some(TypeBinding::Type(lowered));
        }
        let mut item_stack = Vec::new();
        self.partial_type_constructor_binding(ty, &mut item_stack)
            .map(TypeBinding::Constructor)
    }

    pub(crate) fn open_poly_type_binding(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<TypeBinding> {
        self.instantiate_poly_type_binding(ty, bindings)
            .or_else(|| {
                self.lower_open_annotation(ty)
                    .map(TypeBinding::Type)
                    .or_else(|| {
                        let mut item_stack = Vec::new();
                        self.partial_type_constructor_binding(ty, &mut item_stack)
                            .map(TypeBinding::Constructor)
                    })
            })
    }

    pub(crate) fn instantiate_poly_hir_type(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type(ty, bindings, &mut Vec::new())
    }

    pub(crate) fn instantiate_poly_hir_type_partially(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type_partially(ty, bindings, &mut Vec::new())
    }

    pub(crate) fn lower_poly_type_partially(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return Some(lowered);
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.get(parameter) {
                        Some(TypeBinding::Type(ty)) => Some(ty.clone()),
                        Some(TypeBinding::Constructor(binding)) => {
                            let arity = type_constructor_arity(binding.head, self.module);
                            (binding.arguments.len() == arity)
                                .then(|| {
                                    self.apply_type_constructor(
                                        binding.head,
                                        &binding.arguments,
                                        item_stack,
                                    )
                                })
                                .flatten()
                        }
                        None => Some(GateType::TypeParameter {
                            parameter: *parameter,
                            name: self.module.type_parameters()[*parameter]
                                .name
                                .text()
                                .to_owned(),
                        }),
                    }
                }
                ResolutionState::Resolved(TypeResolution::Builtin(
                    builtin @ (BuiltinType::Int
                    | BuiltinType::Float
                    | BuiltinType::Decimal
                    | BuiltinType::BigInt
                    | BuiltinType::Bool
                    | BuiltinType::Text
                    | BuiltinType::Unit
                    | BuiltinType::Bytes),
                )) => Some(GateType::Primitive(*builtin)),
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    self.lower_type_item(*item_id, &[], item_stack, true)
                }
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type_partially(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type_partially(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => self
                .lower_poly_record_row_transform_partially(
                    transform, *source, bindings, item_stack,
                ),
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(
                    self.lower_poly_type_partially(*parameter, bindings, item_stack)?,
                ),
                result: Box::new(self.lower_poly_type_partially(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(
                                    binding.head,
                                    &all_arguments,
                                    item_stack,
                                )
                            })
                            .flatten()
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() != arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        self.apply_type_constructor(head, &lowered_arguments, item_stack)
                    }
                }
            }
        }
    }

    pub(crate) fn match_poly_hir_type(
        &mut self,
        ty: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        self.match_poly_hir_type_inner(ty, actual, bindings, &mut Vec::new())
    }

    pub(crate) fn match_poly_type_binding(
        &mut self,
        ty: TypeId,
        actual: &TypeBinding,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        if let Some(candidate) = self.instantiate_poly_type_binding(ty, bindings) {
            return candidate.matches(actual);
        }
        match (&self.module.types()[ty].kind, actual) {
            (TypeKind::Name(reference), _) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            (TypeKind::Apply { callee, arguments }, TypeBinding::Constructor(actual_binding)) => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return false;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Item(_)) => {
                        let Some((head, _)) = self.type_constructor_head_and_arity(*callee) else {
                            return false;
                        };
                        if head != actual_binding.head()
                            || arguments.len() != actual_binding.arguments.len()
                        {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments.iter().zip(actual_binding.arguments.iter()).all(
                            |(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            },
                        )
                    }
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let Some(prefix_len) =
                            actual_binding.arguments.len().checked_sub(arguments.len())
                        else {
                            return false;
                        };
                        let prefix = TypeBinding::Constructor(TypeConstructorBinding {
                            head: actual_binding.head(),
                            arguments: actual_binding.arguments[..prefix_len].to_vec(),
                        });
                        let matches_prefix = match bindings.entry(*parameter) {
                            Entry::Occupied(entry) => entry.get().matches(&prefix),
                            Entry::Vacant(entry) => {
                                entry.insert(prefix);
                                true
                            }
                        };
                        if !matches_prefix {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments
                            .iter()
                            .zip(actual_binding.arguments[prefix_len..].iter())
                            .all(|(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            })
                    }
                    _ => false,
                }
            }
            (TypeKind::Tuple(_), _)
            | (TypeKind::Record(_), _)
            | (TypeKind::RecordTransform { .. }, _)
            | (TypeKind::Arrow { .. }, _)
            | (TypeKind::Apply { .. }, TypeBinding::Type(_)) => false,
        }
    }

    pub(crate) fn recurrence_target_hint_for_annotation(
        &mut self,
        annotation: TypeId,
    ) -> Option<RecurrenceTargetHint> {
        let ty = self.lower_annotation(annotation)?;
        Some(match ty.recurrence_target_evidence() {
            Some(evidence) => RecurrenceTargetHint::Evidence(evidence),
            None => RecurrenceTargetHint::UnsupportedType {
                ty,
                span: self.module.types()[annotation].span,
            },
        })
    }

    pub(crate) fn item_value_type(&mut self, item_id: ItemId) -> Option<GateType> {
        if let Some(cached) = self.item_types.get(&item_id) {
            return cached.clone();
        }
        self.item_types.insert(item_id, None);
        let ty = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| self.infer_expr(item.body, &GateExprEnv::default(), None).ty),
            Item::Function(item) => {
                let explicit_signature = item
                    .annotation
                    .and_then(|annotation| self.lower_open_annotation(annotation));
                let explicit_parameter_types = explicit_signature
                    .as_ref()
                    .and_then(|ty| Self::arrow_parameter_types(ty, item.parameters.len()));
                let explicit_result = explicit_signature.as_ref().and_then(|ty| {
                    Self::arrow_result_type(ty, item.parameters.len()).or_else(|| Some(ty.clone()))
                });
                let mut env = GateExprEnv::default();
                let mut parameters = Vec::with_capacity(item.parameters.len());
                for parameter in &item.parameters {
                    let parameter_ty = match parameter.annotation {
                        Some(annotation) => self.lower_open_annotation(annotation)?,
                        None => {
                            if let Some(parameter_types) = explicit_parameter_types.as_ref() {
                                parameter_types.get(parameters.len())?.clone()
                            } else {
                                if !self.allow_function_inference
                                    || !supports_same_module_function_inference(item)
                                {
                                    return None;
                                }
                                let inferred = self.inferred_function_types().get(&item_id).cloned()?;
                                let parameter_types =
                                    Self::arrow_parameter_types(&inferred, item.parameters.len())?;
                                parameter_types.get(parameters.len())?.clone()
                            }
                        }
                    };
                    env.locals.insert(parameter.binding, parameter_ty.clone());
                    parameters.push(parameter_ty);
                }
                let result = explicit_result.or_else(|| {
                        if self.allow_function_inference
                            && supports_same_module_function_inference(item)
                        {
                            let inferred = self.inferred_function_types().get(&item_id).cloned();
                            inferred
                                .and_then(|ty| Self::arrow_result_type(&ty, item.parameters.len()))
                        } else {
                            self.infer_expr(item.body, &env, None).ty
                        }
                    })?;
                let mut ty = result;
                for parameter in parameters.into_iter().rev() {
                    ty = GateType::Arrow {
                        parameter: Box::new(parameter),
                        result: Box::new(ty),
                    };
                }
                Some(ty)
            }
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    let body_ty = self.infer_expr(body, &GateExprEnv::default(), None).ty?;
                    // Signal pipe propagation already wraps the body type in
                    // Signal; avoid double-wrapping Signal(Signal(T)).
                    Some(if matches!(body_ty, GateType::Signal(_)) {
                        body_ty
                    } else {
                        GateType::Signal(Box::new(body_ty))
                    })
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        self.item_types.insert(item_id, ty.clone());
        ty
    }

    pub(crate) fn item_value_actual(&mut self, item_id: ItemId) -> Option<SourceOptionActualType> {
        if let Some(cached) = self.item_actuals.get(&item_id) {
            return cached.clone();
        }
        self.item_actuals.insert(item_id, None);
        let actual = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    self.infer_expr(item.body, &GateExprEnv::default(), None)
                        .actual()
                }),
            Item::Function(_) => self
                .item_value_type(item_id)
                .map(|ty| SourceOptionActualType::from_gate_type(&ty)),
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    let body_actual = self
                        .infer_expr(body, &GateExprEnv::default(), None)
                        .actual()?;
                    // Signal pipe propagation already wraps the body actual in
                    // Signal; avoid double-wrapping Signal(Signal(T)).
                    Some(
                        if matches!(body_actual, SourceOptionActualType::Signal(_)) {
                            body_actual
                        } else {
                            SourceOptionActualType::Signal(Box::new(body_actual))
                        },
                    )
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        self.item_actuals.insert(item_id, actual.clone());
        actual
    }

    pub(crate) fn finalize_expr_info(&self, mut info: GateExprInfo) -> GateExprInfo {
        if let Some(ty) = info.ty.as_ref() {
            let actual_matches_ty = info
                .actual
                .as_ref()
                .and_then(SourceOptionActualType::to_gate_type)
                .is_some_and(|actual| actual.same_shape(ty));
            if !actual_matches_ty {
                info.actual = Some(SourceOptionActualType::from_gate_type(ty));
            }
        }
        info.contains_signal |= info.ty.as_ref().is_some_and(GateType::is_signal)
            || info
                .actual
                .as_ref()
                .is_some_and(SourceOptionActualType::is_signal);
        info
    }

    fn maybe_use_ambient_signal_payload(
        &self,
        expr_id: ExprId,
        ambient: Option<&GateType>,
        mut info: GateExprInfo,
    ) -> GateExprInfo {
        if ambient.is_some()
            && matches!(
                self.module.exprs()[expr_id].kind,
                ExprKind::Name(_) | ExprKind::Apply { .. }
            )
            && let Some(GateType::Signal(payload)) = info.ty.clone() {
                info.ty = Some(*payload);
                info.actual = None;
            }
        info
    }

    pub(crate) fn import_value_type(&self, import_id: ImportId) -> Option<GateType> {
        let import = &self.module.imports()[import_id];
        if let Some(ty) = import.callable_type.as_ref() {
            return Some(self.lower_import_value_type(ty));
        }
        match &import.metadata {
            ImportBindingMetadata::Value { ty }
            | ImportBindingMetadata::IntrinsicValue { ty, .. } => {
                Some(self.lower_import_value_type(ty))
            }
            ImportBindingMetadata::InstanceMember { ty, .. } => {
                Some(self.lower_import_value_type(ty))
            }
            ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::Domain { .. }
            | ImportBindingMetadata::DomainSuffix { .. }
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::BuiltinType(_)
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_)
            | ImportBindingMetadata::Unknown => None,
        }
    }

    /// Like `import_value_type`, but also resolves `AmbientValue` imports by
    /// looking up the prelude item they reference.  Used in pipe stage inference
    /// where we need the type of functions like `list.map` (which is registered
    /// as `AmbientValue { name: "__aivi_list_map" }` rather than carrying an
    /// explicit `ImportValueType`).
    pub(crate) fn import_value_type_with_ambient(
        &mut self,
        import_id: ImportId,
    ) -> Option<GateType> {
        if let Some(ty) = self.import_value_type(import_id) {
            return Some(ty);
        }
        // For AmbientValue imports, look up the prelude item by name.
        let ambient_name: String = match &self.module.imports()[import_id].metadata {
            ImportBindingMetadata::AmbientValue { name } => name.to_string(),
            _ => return None,
        };
        let item_id = self
            .module
            .items()
            .iter()
            .find_map(|(id, item)| match item {
                Item::Function(f) if f.name.text() == ambient_name => Some(id),
                Item::Value(v) if v.name.text() == ambient_name => Some(id),
                _ => None,
            })?;
        self.item_value_type(item_id)
    }

    fn imported_type_definition(&self, import: ImportId) -> Option<Box<ImportTypeDefinition>> {
        match &self.module.imports()[import].metadata {
            ImportBindingMetadata::TypeConstructor {
                definition: Some(definition),
                ..
            } => Some(Box::new(definition.clone())),
            _ => None,
        }
    }

    fn opaque_import_type(
        &self,
        import: ImportId,
        name: String,
        arguments: Vec<GateType>,
    ) -> GateType {
        GateType::OpaqueImport {
            import,
            name,
            arguments,
            definition: self.imported_type_definition(import),
        }
    }

    /// When an import carries `ImportBindingMetadata::Domain`, look up the
    /// matching local or ambient `Item::Domain` and return `GateType::Domain`
    /// instead of `GateType::OpaqueImport`.  Falls back to the opaque import
    /// path when no local domain item is found.
    fn import_type_for_domain_or_opaque(
        &self,
        import_id: ImportId,
        name: String,
        arguments: Vec<GateType>,
    ) -> GateType {
        let binding = &self.module.imports()[import_id];
        if matches!(&binding.metadata, ImportBindingMetadata::Domain { .. }) {
            let domain_item = self
                .module
                .root_items()
                .iter()
                .chain(self.module.ambient_items().iter())
                .copied()
                .find(|&id| matches!(&self.module.items()[id], Item::Domain(d) if d.name.text() == name));
            if let Some(item_id) = domain_item {
                return GateType::Domain {
                    item: item_id,
                    name,
                    arguments,
                };
            }
        }
        self.opaque_import_type(import_id, name, arguments)
    }

    pub(crate) fn lower_import_value_type(&self, ty: &ImportValueType) -> GateType {
        match ty {
            ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
            ImportValueType::Tuple(elements) => GateType::Tuple(
                elements
                    .iter()
                    .map(|element| self.lower_import_value_type(element))
                    .collect(),
            ),
            ImportValueType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .map(|field| GateRecordField {
                        name: field.name.to_string(),
                        ty: self.lower_import_value_type(&field.ty),
                    })
                    .collect(),
            ),
            ImportValueType::Arrow { parameter, result } => GateType::Arrow {
                parameter: Box::new(self.lower_import_value_type(parameter)),
                result: Box::new(self.lower_import_value_type(result)),
            },
            ImportValueType::List(element) => {
                GateType::List(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Map { key, value } => GateType::Map {
                key: Box::new(self.lower_import_value_type(key)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Set(element) => {
                GateType::Set(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Option(element) => {
                GateType::Option(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Result { error, value } => GateType::Result {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Validation { error, value } => GateType::Validation {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Signal(element) => {
                GateType::Signal(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Task { error, value } => GateType::Task {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::TypeVariable { index, name } => GateType::TypeParameter {
                parameter: TypeParameterId::from_raw(u32::MAX - *index as u32),
                name: name.clone(),
            },
            ImportValueType::Named {
                type_name,
                arguments,
                definition,
            } => {
                let lowered_args: Vec<GateType> = arguments
                    .iter()
                    .map(|arg| self.lower_import_value_type(arg))
                    .collect();
                // Find the import that provides this type name.
                let import_id = self
                    .module
                    .imports()
                    .iter()
                    .find(|(_, binding)| {
                        binding.imported_name.text() == type_name
                            && matches!(
                                &binding.metadata,
                                ImportBindingMetadata::TypeConstructor { .. }
                                    | ImportBindingMetadata::AmbientType
                                    | ImportBindingMetadata::BuiltinType(_)
                                    | ImportBindingMetadata::Domain { .. }
                            )
                    })
                    .map(|(id, _)| id);
                if let Some(import) = import_id {
                    self.import_type_for_domain_or_opaque(import, type_name.clone(), lowered_args)
                } else {
                    // Fallback: create an opaque import with a sentinel; the type checker
                    // will treat this as an unknown opaque type.
                    GateType::OpaqueImport {
                        import: ImportId::from_raw(u32::MAX),
                        name: type_name.clone(),
                        arguments: lowered_args,
                        definition: definition.clone(),
                    }
                }
            }
        }
    }

    pub(crate) fn intrinsic_value_type(&self, value: IntrinsicValue) -> GateType {
        fn primitive(builtin: BuiltinType) -> GateType {
            GateType::Primitive(builtin)
        }

        fn synthetic_type_parameter(index: usize) -> GateType {
            GateType::TypeParameter {
                parameter: TypeParameterId::from_raw(u32::MAX - index as u32),
                name: format!("T{}", index + 1),
            }
        }

        fn arrow(parameter: GateType, result: GateType) -> GateType {
            GateType::Arrow {
                parameter: Box::new(parameter),
                result: Box::new(result),
            }
        }

        fn task(error: GateType, value: GateType) -> GateType {
            GateType::Task {
                error: Box::new(error),
                value: Box::new(value),
            }
        }

        fn option(element: GateType) -> GateType {
            GateType::Option(Box::new(element))
        }

        fn list(element: GateType) -> GateType {
            GateType::List(Box::new(element))
        }

        fn map(key: GateType, value: GateType) -> GateType {
            GateType::Map {
                key: Box::new(key),
                value: Box::new(value),
            }
        }

        fn record(fields: Vec<(&str, GateType)>) -> GateType {
            GateType::Record(
                fields
                    .into_iter()
                    .map(|(name, ty)| GateRecordField {
                        name: name.to_owned(),
                        ty,
                    })
                    .collect(),
            )
        }

        fn db_connection_type() -> GateType {
            record(vec![("database", primitive(BuiltinType::Text))])
        }

        fn db_param_type() -> GateType {
            record(vec![
                ("kind", primitive(BuiltinType::Text)),
                ("bool", option(primitive(BuiltinType::Bool))),
                ("int", option(primitive(BuiltinType::Int))),
                ("float", option(primitive(BuiltinType::Float))),
                ("decimal", option(primitive(BuiltinType::Decimal))),
                ("bigInt", option(primitive(BuiltinType::BigInt))),
                ("text", option(primitive(BuiltinType::Text))),
                ("bytes", option(primitive(BuiltinType::Bytes))),
            ])
        }

        fn db_statement_type() -> GateType {
            record(vec![
                ("sql", primitive(BuiltinType::Text)),
                ("arguments", list(db_param_type())),
            ])
        }

        fn db_rows_type() -> GateType {
            list(map(
                primitive(BuiltinType::Text),
                primitive(BuiltinType::Text),
            ))
        }

        match value {
            IntrinsicValue::TupleConstructor { arity } => {
                let elements: Vec<_> = (0..arity).map(synthetic_type_parameter).collect();
                let mut ty = GateType::Tuple(elements.clone());
                for element in elements.into_iter().rev() {
                    ty = arrow(element, ty);
                }
                ty
            }
            IntrinsicValue::CustomCapabilityCommand(_) => {
                unreachable!("custom capability commands should type through synthetic imports")
            }
            IntrinsicValue::RandomBytes => arrow(
                primitive(BuiltinType::Int),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::RandomInt => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
                ),
            ),
            IntrinsicValue::StdoutWrite | IntrinsicValue::StderrWrite => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::FsWriteText => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsWriteBytes => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Bytes),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsCreateDirAll | IntrinsicValue::FsDeleteFile => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::DbParamBool => arrow(primitive(BuiltinType::Bool), db_param_type()),
            IntrinsicValue::DbParamInt => arrow(primitive(BuiltinType::Int), db_param_type()),
            IntrinsicValue::DbParamFloat => arrow(primitive(BuiltinType::Float), db_param_type()),
            IntrinsicValue::DbParamDecimal => {
                arrow(primitive(BuiltinType::Decimal), db_param_type())
            }
            IntrinsicValue::DbParamBigInt => arrow(primitive(BuiltinType::BigInt), db_param_type()),
            IntrinsicValue::DbParamText => arrow(primitive(BuiltinType::Text), db_param_type()),
            IntrinsicValue::DbParamBytes => arrow(primitive(BuiltinType::Bytes), db_param_type()),
            IntrinsicValue::DbStatement => arrow(
                primitive(BuiltinType::Text),
                arrow(list(db_param_type()), db_statement_type()),
            ),
            IntrinsicValue::DbQuery => arrow(
                db_connection_type(),
                arrow(
                    db_statement_type(),
                    task(primitive(BuiltinType::Text), db_rows_type()),
                ),
            ),
            IntrinsicValue::DbCommit => arrow(
                db_connection_type(),
                arrow(
                    list(primitive(BuiltinType::Text)),
                    arrow(
                        list(db_statement_type()),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                    ),
                ),
            ),
            IntrinsicValue::FloatFloor
            | IntrinsicValue::FloatCeil
            | IntrinsicValue::FloatRound
            | IntrinsicValue::FloatSqrt
            | IntrinsicValue::FloatAbs => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatToInt => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Int))
            }
            IntrinsicValue::FloatFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatToText => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Text))
            }
            IntrinsicValue::FloatParseText => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Float))),
            ),
            IntrinsicValue::FsReadText => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::FsReadDir => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    GateType::List(Box::new(primitive(BuiltinType::Text))),
                ),
            ),
            IntrinsicValue::FsExists => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::FsReadBytes => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::FsRename => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsCopy => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsDeleteDir => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::PathParent => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathFilename => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathStem => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathExtension => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathJoin => arrow(
                primitive(BuiltinType::Text),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::PathIsAbsolute => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bool))
            }
            IntrinsicValue::PathNormalize => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::BytesLength => {
                arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Int))
            }
            IntrinsicValue::BytesGet => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Bytes),
                    GateType::Option(Box::new(primitive(BuiltinType::Int))),
                ),
            ),
            IntrinsicValue::BytesSlice => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Bytes)),
                ),
            ),
            IntrinsicValue::BytesAppend => arrow(
                primitive(BuiltinType::Bytes),
                arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::BytesFromText => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes))
            }
            IntrinsicValue::BytesToText => arrow(
                primitive(BuiltinType::Bytes),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::BytesRepeat => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::BytesEmpty => primitive(BuiltinType::Bytes),
            IntrinsicValue::JsonValidate => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::JsonGet => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        GateType::Option(Box::new(primitive(BuiltinType::Text))),
                    ),
                ),
            ),
            IntrinsicValue::JsonAt => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Int),
                    task(
                        primitive(BuiltinType::Text),
                        GateType::Option(Box::new(primitive(BuiltinType::Text))),
                    ),
                ),
            ),
            IntrinsicValue::JsonKeys => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    GateType::List(Box::new(primitive(BuiltinType::Text))),
                ),
            ),
            IntrinsicValue::JsonPretty => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::JsonMinify => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::XdgDataHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgConfigHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgCacheHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgStateHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgRuntimeDir => {
                GateType::Option(Box::new(primitive(BuiltinType::Text)))
            }
            IntrinsicValue::XdgDataDirs => GateType::List(Box::new(primitive(BuiltinType::Text))),
            IntrinsicValue::XdgConfigDirs => GateType::List(Box::new(primitive(BuiltinType::Text))),
            // Text intrinsics
            IntrinsicValue::TextLength | IntrinsicValue::TextByteLen => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Int))
            }
            IntrinsicValue::TextSlice => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextFind => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    option(primitive(BuiltinType::Int)),
                ),
            ),
            IntrinsicValue::TextContains
            | IntrinsicValue::TextStartsWith
            | IntrinsicValue::TextEndsWith => arrow(
                primitive(BuiltinType::Text),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::TextToUpper
            | IntrinsicValue::TextToLower
            | IntrinsicValue::TextTrim
            | IntrinsicValue::TextTrimStart
            | IntrinsicValue::TextTrimEnd => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextReplace | IntrinsicValue::TextReplaceAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextSplit => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    list(primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextRepeat => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::TextFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextParseInt => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::TextFromBool => {
                arrow(primitive(BuiltinType::Bool), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextParseBool => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::TextConcat => arrow(
                list(primitive(BuiltinType::Text)),
                primitive(BuiltinType::Text),
            ),
            // Float transcendental intrinsics
            IntrinsicValue::FloatSin
            | IntrinsicValue::FloatCos
            | IntrinsicValue::FloatTan
            | IntrinsicValue::FloatAtan
            | IntrinsicValue::FloatExp
            | IntrinsicValue::FloatTrunc
            | IntrinsicValue::FloatFrac => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatAsin | IntrinsicValue::FloatAcos => arrow(
                primitive(BuiltinType::Float),
                option(primitive(BuiltinType::Float)),
            ),
            IntrinsicValue::FloatLog | IntrinsicValue::FloatLog2 | IntrinsicValue::FloatLog10 => {
                arrow(
                    primitive(BuiltinType::Float),
                    option(primitive(BuiltinType::Float)),
                )
            }
            IntrinsicValue::FloatAtan2 | IntrinsicValue::FloatHypot => arrow(
                primitive(BuiltinType::Float),
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float)),
            ),
            IntrinsicValue::FloatPow => arrow(
                primitive(BuiltinType::Float),
                arrow(
                    primitive(BuiltinType::Float),
                    option(primitive(BuiltinType::Float)),
                ),
            ),
            // Time intrinsics
            IntrinsicValue::TimeNowMs | IntrinsicValue::TimeMonotonicMs => {
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Int))
            }
            IntrinsicValue::TimeFormat => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TimeParse => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
                ),
            ),
            // Env intrinsics
            IntrinsicValue::EnvGet => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    option(primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::EnvList => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    list(GateType::Tuple(vec![
                        primitive(BuiltinType::Text),
                        primitive(BuiltinType::Text),
                    ])),
                ),
            ),
            // Log intrinsics
            IntrinsicValue::LogEmit => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::LogEmitContext => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        list(GateType::Tuple(vec![
                            primitive(BuiltinType::Text),
                            primitive(BuiltinType::Text),
                        ])),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                    ),
                ),
            ),
            // Random float intrinsic
            IntrinsicValue::RandomFloat => {
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Float))
            }
            // I18n intrinsics
            IntrinsicValue::I18nTranslate => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::I18nTranslatePlural => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Text)),
                ),
            ),
            // Regex intrinsics
            IntrinsicValue::RegexIsMatch => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
                ),
            ),
            IntrinsicValue::RegexFind => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        option(primitive(BuiltinType::Int)),
                    ),
                ),
            ),
            IntrinsicValue::RegexFindText => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        option(primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::RegexFindAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        list(primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::RegexReplace | IntrinsicValue::RegexReplaceAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        primitive(BuiltinType::Text),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::HttpGet | IntrinsicValue::HttpDelete => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::HttpGetBytes => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::HttpGetStatus => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::HttpPost | IntrinsicValue::HttpPut => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        primitive(BuiltinType::Text),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::HttpPostJson => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::HttpHead => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    list(GateType::Tuple(vec![
                        primitive(BuiltinType::Text),
                        primitive(BuiltinType::Text),
                    ])),
                ),
            ),
            IntrinsicValue::BigIntFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::BigInt))
            }
            IntrinsicValue::BigIntFromText => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::BigInt)),
            ),
            IntrinsicValue::BigIntToInt => arrow(
                primitive(BuiltinType::BigInt),
                option(primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::BigIntToText => {
                arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Text))
            }
            IntrinsicValue::BigIntAdd | IntrinsicValue::BigIntSub | IntrinsicValue::BigIntMul => {
                arrow(
                    primitive(BuiltinType::BigInt),
                    arrow(
                        primitive(BuiltinType::BigInt),
                        primitive(BuiltinType::BigInt),
                    ),
                )
            }
            IntrinsicValue::BigIntDiv | IntrinsicValue::BigIntMod => arrow(
                primitive(BuiltinType::BigInt),
                arrow(
                    primitive(BuiltinType::BigInt),
                    option(primitive(BuiltinType::BigInt)),
                ),
            ),
            IntrinsicValue::BigIntPow => arrow(
                primitive(BuiltinType::BigInt),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::BigInt)),
            ),
            IntrinsicValue::BigIntNeg | IntrinsicValue::BigIntAbs => arrow(
                primitive(BuiltinType::BigInt),
                primitive(BuiltinType::BigInt),
            ),
            IntrinsicValue::BigIntCmp => arrow(
                primitive(BuiltinType::BigInt),
                arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::BigIntEq | IntrinsicValue::BigIntGt | IntrinsicValue::BigIntLt => {
                arrow(
                    primitive(BuiltinType::BigInt),
                    arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Bool)),
                )
            }
            IntrinsicValue::BitAnd
            | IntrinsicValue::BitOr
            | IntrinsicValue::BitXor
            | IntrinsicValue::ShiftLeft
            | IntrinsicValue::ShiftRight
            | IntrinsicValue::ShiftRightUnsigned => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::BitNot => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Int))
            }
            IntrinsicValue::IntAdd
            | IntrinsicValue::IntSub
            | IntrinsicValue::IntMul
            | IntrinsicValue::IntDiv
            | IntrinsicValue::IntMod => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::IntNeg => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Int))
            }
        }
    }

    pub(crate) fn domain_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<DomainMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Resolved(TermResolution::DomainConstructor(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn domain_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.domain_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.domain_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn infer_domain_member_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let candidates = self.domain_member_candidates(reference)?;
        if candidates.len() != 1 {
            return None;
        }
        self.lower_domain_member_annotation(candidates[0], &HashMap::new())
    }

    pub(crate) fn select_domain_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<DomainMemberSelection<GateType>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_name_candidate(resolution, expected)
            }),
        )
    }

    pub(crate) fn select_domain_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<DomainMemberCallMatch>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_call_candidate(resolution, argument_types, expected_result)
            }),
        )
    }

    pub(crate) fn match_item_name_candidate(
        &mut self,
        item_id: ItemId,
        expected: &GateType,
    ) -> Option<GateType> {
        let annotation = match &self.module.items()[item_id] {
            Item::Value(item) => item.annotation?,
            Item::Function(item) => item.annotation?,
            Item::Signal(item) => item.annotation?,
            _ => return None,
        };
        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !self.match_hir_type(annotation, expected, &mut substitutions, &mut item_stack) {
            return None;
        }
        let lowered = self.lower_type(annotation, &substitutions, &mut item_stack, true)?;
        lowered.same_shape(expected).then_some(lowered)
    }

    fn match_gate_type_template(
        template: &GateType,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
    ) -> bool {
        if let Some(expanded_template) = template.expand_transparent_import_alias() {
            return Self::match_gate_type_template(&expanded_template, actual, substitutions);
        }
        if let Some(expanded_actual) = actual.expand_transparent_import_alias() {
            return Self::match_gate_type_template(template, &expanded_actual, substitutions);
        }
        match template {
            GateType::TypeParameter { parameter, .. } => match substitutions.entry(*parameter) {
                Entry::Occupied(existing) => existing.get().same_shape(actual),
                Entry::Vacant(slot) => {
                    slot.insert(actual.clone());
                    true
                }
            },
            GateType::Primitive(_) => template == actual,
            GateType::Tuple(template_elements) => match actual {
                GateType::Tuple(actual_elements) => {
                    template_elements.len() == actual_elements.len()
                        && template_elements
                            .iter()
                            .zip(actual_elements.iter())
                            .all(|(template, actual)| {
                                Self::match_gate_type_template(template, actual, substitutions)
                            })
                }
                _ => false,
            },
            GateType::Record(template_fields) => match actual {
                GateType::Record(actual_fields) => {
                    template_fields.len() == actual_fields.len()
                        && template_fields
                            .iter()
                            .zip(actual_fields.iter())
                            .all(|(template, actual)| {
                                template.name == actual.name
                                    && Self::match_gate_type_template(
                                        &template.ty,
                                        &actual.ty,
                                        substitutions,
                                    )
                            })
                }
                _ => false,
            },
            GateType::Arrow {
                parameter: template_parameter,
                result: template_result,
            } => match actual {
                GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } => {
                    Self::match_gate_type_template(
                        template_parameter,
                        actual_parameter,
                        substitutions,
                    ) && Self::match_gate_type_template(
                        template_result,
                        actual_result,
                        substitutions,
                    )
                }
                _ => false,
            },
            GateType::List(template_element) => match actual {
                GateType::List(actual_element) => {
                    Self::match_gate_type_template(template_element, actual_element, substitutions)
                }
                _ => false,
            },
            GateType::Map {
                key: template_key,
                value: template_value,
            } => match actual {
                GateType::Map {
                    key: actual_key,
                    value: actual_value,
                } => {
                    Self::match_gate_type_template(template_key, actual_key, substitutions)
                        && Self::match_gate_type_template(
                            template_value,
                            actual_value,
                            substitutions,
                        )
                }
                _ => false,
            },
            GateType::Set(template_element) => match actual {
                GateType::Set(actual_element) => {
                    Self::match_gate_type_template(template_element, actual_element, substitutions)
                }
                _ => false,
            },
            GateType::Option(template_element) => match actual {
                GateType::Option(actual_element) => {
                    Self::match_gate_type_template(template_element, actual_element, substitutions)
                }
                _ => false,
            },
            GateType::Result {
                error: template_error,
                value: template_value,
            } => match actual {
                GateType::Result {
                    error: actual_error,
                    value: actual_value,
                } => {
                    Self::match_gate_type_template(template_error, actual_error, substitutions)
                        && Self::match_gate_type_template(
                            template_value,
                            actual_value,
                            substitutions,
                        )
                }
                _ => false,
            },
            GateType::Validation {
                error: template_error,
                value: template_value,
            } => match actual {
                GateType::Validation {
                    error: actual_error,
                    value: actual_value,
                } => {
                    Self::match_gate_type_template(template_error, actual_error, substitutions)
                        && Self::match_gate_type_template(
                            template_value,
                            actual_value,
                            substitutions,
                        )
                }
                _ => false,
            },
            GateType::Signal(template_element) => match actual {
                GateType::Signal(actual_element) => {
                    Self::match_gate_type_template(template_element, actual_element, substitutions)
                }
                _ => false,
            },
            GateType::Task {
                error: template_error,
                value: template_value,
            } => match actual {
                GateType::Task {
                    error: actual_error,
                    value: actual_value,
                } => {
                    Self::match_gate_type_template(template_error, actual_error, substitutions)
                        && Self::match_gate_type_template(
                            template_value,
                            actual_value,
                            substitutions,
                        )
                }
                _ => false,
            },
            GateType::Domain {
                item,
                arguments: template_arguments,
                ..
            } => match actual {
                GateType::Domain {
                    item: actual_item,
                    arguments: actual_arguments,
                    ..
                } => {
                    item == actual_item
                        && template_arguments.len() == actual_arguments.len()
                        && template_arguments
                            .iter()
                            .zip(actual_arguments.iter())
                            .all(|(template, actual)| {
                                Self::match_gate_type_template(template, actual, substitutions)
                            })
                }
                _ => false,
            },
            GateType::OpaqueItem {
                item,
                arguments: template_arguments,
                ..
            } => match actual {
                GateType::OpaqueItem {
                    item: actual_item,
                    arguments: actual_arguments,
                    ..
                } => {
                    item == actual_item
                        && template_arguments.len() == actual_arguments.len()
                        && template_arguments
                            .iter()
                            .zip(actual_arguments.iter())
                            .all(|(template, actual)| {
                                Self::match_gate_type_template(template, actual, substitutions)
                            })
                }
                _ => false,
            },
            GateType::OpaqueImport {
                import,
                arguments: template_arguments,
                ..
            } => match actual {
                GateType::OpaqueImport {
                    import: actual_import,
                    arguments: actual_arguments,
                    ..
                } => {
                    import == actual_import
                        && template_arguments.len() == actual_arguments.len()
                        && template_arguments
                            .iter()
                            .zip(actual_arguments.iter())
                            .all(|(template, actual)| {
                                Self::match_gate_type_template(template, actual, substitutions)
                            })
                }
                _ => false,
            },
        }
    }

    fn specialize_gate_type_template(
        &self,
        template: &GateType,
        expected: &GateType,
    ) -> Option<GateType> {
        let mut substitutions = HashMap::new();
        Self::match_gate_type_template(template, expected, &mut substitutions)
            .then(|| template.substitute_type_parameters(&substitutions))
            .filter(|specialized| specialized.same_shape(expected))
    }

    pub(crate) fn class_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<ClassMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::ClassMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::DomainConstructor(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn class_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.class_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.class_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn hoisted_import_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<ImportId>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            _ => None,
        }
    }

    /// Try to select a unique hoisted import candidate by comparing each
    /// candidate's value type against `expected`.  Returns the resolved
    /// `GateRuntimeReference::Import` on success, or the diagnostic
    /// context on failure.
    pub(crate) fn select_hoisted_import(
        &mut self,
        reference: &TermReference,
        expected: Option<&GateType>,
    ) -> Option<ImportId> {
        let candidates = self.hoisted_import_candidates(reference)?;
        let mut matches = Vec::new();
        for import_id in candidates {
            if let Some(ty) = self.import_value_type_with_ambient(import_id) {
                let fits = match expected {
                    Some(expected) => expected.fits_template(&ty),
                    None => true,
                };
                if fits {
                    matches.push(import_id);
                }
            }
        }
        match matches.len() {
            1 => matches.pop(),
            _ => None,
        }
    }

    pub(crate) fn select_class_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<ClassMemberCallMatch>> {
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) =
                self.match_class_member_call_candidate(candidate, argument_types, expected_result)
            {
                matches.push(matched);
            }
        }
        Some(match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one class member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        })
    }

    pub(crate) fn match_class_member_call_candidate(
        &mut self,
        resolution: ClassMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<ClassMemberCallMatch> {
        let (class_parameter, member_annotation, member_context) =
            self.class_member_signature(resolution)?;
        let mut bindings = PolyTypeBindings::new();
        let mut current = member_annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            if !self.match_poly_hir_type(parameter, argument, &mut bindings) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result
            && !self.match_poly_hir_type(current, expected, &mut bindings)
        {
            return None;
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            parameters.push(self.instantiate_poly_hir_type(parameter, &bindings)?);
        }
        let result = self.instantiate_poly_hir_type(current, &bindings)?;
        if let Some(expected) = expected_result
            && !result.same_shape(expected)
        {
            return None;
        }

        let evidence = ClassConstraintBinding {
            class_item: resolution.class,
            subject: bindings.get(&class_parameter)?.clone(),
        };
        let constraints = member_context
            .iter()
            .map(|constraint| self.class_constraint_binding(*constraint, &bindings))
            .collect::<Option<Vec<_>>>()?;
        Some(ClassMemberCallMatch {
            resolution,
            parameters,
            result,
            evidence,
            constraints,
        })
    }

    pub(crate) fn class_member_signature(
        &self,
        resolution: ClassMemberResolution,
    ) -> Option<(TypeParameterId, TypeId, Vec<TypeId>)> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        let context = class_item
            .superclasses
            .iter()
            .chain(class_item.param_constraints.iter())
            .chain(member.context.iter())
            .copied()
            .collect();
        Some((*class_item.parameters.first(), member.annotation, context))
    }

    pub(crate) fn class_member_label(&self, resolution: ClassMemberResolution) -> Option<String> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        Some(format!("{}.{}", class_item.name.text(), member.name.text()))
    }

    pub(crate) fn class_constraint_binding(
        &mut self,
        constraint: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<ClassConstraintBinding> {
        let (class_item, subject) = self.class_constraint_parts(constraint)?;
        Some(ClassConstraintBinding {
            class_item,
            subject: self.instantiate_poly_type_binding(subject, bindings)?,
        })
    }

    pub(crate) fn open_class_constraint_binding(
        &mut self,
        constraint: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<ClassConstraintBinding> {
        let (class_item, subject) = self.class_constraint_parts(constraint)?;
        Some(ClassConstraintBinding {
            class_item,
            subject: self.open_poly_type_binding(subject, bindings)?,
        })
    }

    pub(crate) fn class_constraint_parts(&self, constraint: TypeId) -> Option<(ItemId, TypeId)> {
        let ty = self.module.types().get(constraint)?;
        match &ty.kind {
            TypeKind::Apply { callee, arguments } if arguments.len() == 1 => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
                    reference.resolution.as_ref()
                else {
                    return None;
                };
                matches!(self.module.items()[*item_id], Item::Class(_))
                    .then_some((*item_id, *arguments.first()))
            }
            _ => None,
        }
    }

    pub(crate) fn select_domain_member_candidate<T>(
        &mut self,
        candidates: Vec<DomainMemberResolution>,
        mut matcher: impl FnMut(&mut Self, DomainMemberResolution) -> Option<T>,
    ) -> DomainMemberSelection<T> {
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) = matcher(self, candidate) {
                matches.push(matched);
            }
        }
        match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one domain member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        }
    }

    pub(crate) fn match_domain_member_name_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        expected: &GateType,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !self.match_hir_type(annotation, expected, &mut substitutions, &mut item_stack) {
            return None;
        }
        let lowered = self.lower_domain_member_annotation(resolution, &substitutions)?;
        lowered.same_shape(expected).then_some(lowered)
    }

    pub(crate) fn match_domain_member_call_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberCallMatch> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut current = annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            let mut item_stack = Vec::new();
            if !self.match_hir_type(parameter, argument, &mut substitutions, &mut item_stack) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(current, expected, &mut substitutions, &mut item_stack) {
                return None;
            }
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            let mut item_stack = Vec::new();
            parameters.push(self.lower_type(parameter, &substitutions, &mut item_stack, false)?);
        }
        let mut item_stack = Vec::new();
        let result = self.lower_type(current, &substitutions, &mut item_stack, false)?;
        if let Some(expected) = expected_result
            && !result.same_shape(expected) {
                return None;
            }
        Some(DomainMemberCallMatch { parameters, result })
    }

    pub(crate) fn match_hir_type(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_type(type_id, substitutions, item_stack, false) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match substitutions.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().same_shape(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_hir_type(*element, actual, substitutions, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_hir_type(field.ty, &actual_field.ty, substitutions, item_stack)
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_hir_type(parameter, actual_parameter, substitutions, item_stack)
                    && self.match_hir_type(result, actual_result, substitutions, item_stack)
            }
            TypeKind::Apply { callee, arguments } => self.match_hir_type_application(
                callee,
                &arguments,
                actual,
                substitutions,
                item_stack,
            ),
            TypeKind::RecordTransform { .. } => false,
        }
    }

    pub(crate) fn match_hir_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        let arguments = arguments.iter().copied().collect::<Vec<_>>();
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                let GateType::List(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                let GateType::Map { key, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], key, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                let GateType::Set(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                let GateType::Option(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                let GateType::Result { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                let GateType::Validation { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                let GateType::Signal(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                let GateType::Task { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => match actual {
                GateType::Domain {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                GateType::OpaqueItem {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                _ => false,
            },
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => match actual {
                GateType::OpaqueImport {
                    import,
                    arguments: actual_arguments,
                    ..
                } if *import == *import_id && arguments.len() == actual_arguments.len() => {
                    arguments
                        .iter()
                        .zip(actual_arguments.iter())
                        .all(|(argument, actual)| {
                            self.match_hir_type(*argument, actual, substitutions, item_stack)
                        })
                }
                _ => false,
            },
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_)) => false,
        }
    }

    pub(crate) fn lower_domain_member_annotation(
        &mut self,
        resolution: DomainMemberResolution,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut item_stack = Vec::new();
        self.lower_type(annotation, substitutions, &mut item_stack, false)
    }

    pub(crate) fn lower_domain_member_implementation_type(
        &mut self,
        owner: ItemId,
        kind: DomainMemberKind,
        annotation: TypeId,
    ) -> Option<GateType> {
        let lowered = self.lower_open_annotation(annotation)?;
        if kind == DomainMemberKind::Literal {
            Some(lowered)
        } else {
            Some(self.rewrite_current_domain_carrier_view(owner, &lowered))
        }
    }

    pub(crate) fn domain_member_annotation(
        &self,
        resolution: DomainMemberResolution,
    ) -> Option<TypeId> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        domain
            .members
            .get(resolution.member_index)
            .filter(|member| member.kind == DomainMemberKind::Method)
            .map(|member| member.annotation)
    }

    pub(crate) fn domain_member_label(&self, resolution: DomainMemberResolution) -> Option<String> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        let member = domain.members.get(resolution.member_index)?;
        Some(format!("{}.{}", domain.name.text(), member.name.text()))
    }

    fn rewrite_current_domain_carrier_view(&mut self, owner: ItemId, ty: &GateType) -> GateType {
        match ty {
            GateType::Primitive(_) | GateType::TypeParameter { .. } => ty.clone(),
            GateType::Tuple(elements) => GateType::Tuple(
                elements
                    .iter()
                    .map(|element| self.rewrite_current_domain_carrier_view(owner, element))
                    .collect(),
            ),
            GateType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .map(|field| GateRecordField {
                        name: field.name.clone(),
                        ty: self.rewrite_current_domain_carrier_view(owner, &field.ty),
                    })
                    .collect(),
            ),
            GateType::Arrow { parameter, result } => GateType::Arrow {
                parameter: Box::new(self.rewrite_current_domain_carrier_view(owner, parameter)),
                result: Box::new(self.rewrite_current_domain_carrier_view(owner, result)),
            },
            GateType::List(element) => GateType::List(Box::new(
                self.rewrite_current_domain_carrier_view(owner, element),
            )),
            GateType::Map { key, value } => GateType::Map {
                key: Box::new(self.rewrite_current_domain_carrier_view(owner, key)),
                value: Box::new(self.rewrite_current_domain_carrier_view(owner, value)),
            },
            GateType::Set(element) => GateType::Set(Box::new(
                self.rewrite_current_domain_carrier_view(owner, element),
            )),
            GateType::Option(element) => GateType::Option(Box::new(
                self.rewrite_current_domain_carrier_view(owner, element),
            )),
            GateType::Result { error, value } => GateType::Result {
                error: Box::new(self.rewrite_current_domain_carrier_view(owner, error)),
                value: Box::new(self.rewrite_current_domain_carrier_view(owner, value)),
            },
            GateType::Validation { error, value } => GateType::Validation {
                error: Box::new(self.rewrite_current_domain_carrier_view(owner, error)),
                value: Box::new(self.rewrite_current_domain_carrier_view(owner, value)),
            },
            GateType::Signal(element) => GateType::Signal(Box::new(
                self.rewrite_current_domain_carrier_view(owner, element),
            )),
            GateType::Task { error, value } => GateType::Task {
                error: Box::new(self.rewrite_current_domain_carrier_view(owner, error)),
                value: Box::new(self.rewrite_current_domain_carrier_view(owner, value)),
            },
            GateType::Domain {
                item,
                arguments,
                name: _,
            } if *item == owner => self
                .lower_current_domain_carrier_type(owner, arguments)
                .map(|carrier| self.rewrite_current_domain_carrier_view(owner, &carrier))
                .unwrap_or_else(|| ty.clone()),
            GateType::Domain {
                item,
                name,
                arguments,
            } => GateType::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.rewrite_current_domain_carrier_view(owner, argument))
                    .collect(),
            },
            GateType::OpaqueItem {
                item,
                name,
                arguments,
            } => GateType::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.rewrite_current_domain_carrier_view(owner, argument))
                    .collect(),
            },
            GateType::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => GateType::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.rewrite_current_domain_carrier_view(owner, argument))
                    .collect(),
                definition: definition.clone(),
            },
        }
    }

    fn lower_current_domain_carrier_type(
        &mut self,
        owner: ItemId,
        arguments: &[GateType],
    ) -> Option<GateType> {
        let Item::Domain(domain) = &self.module.items()[owner] else {
            return None;
        };
        let mut carrier = self.lower_open_annotation(domain.carrier)?;
        let substitutions = domain
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        if !substitutions.is_empty() {
            carrier = carrier.substitute_type_parameters(&substitutions);
        }
        Some(carrier)
    }

    pub(crate) fn lower_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => self.lower_type_reference(
                reference,
                substitutions,
                item_stack,
                allow_open_type_parameters,
            ),
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_type(
                        *element,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_type(
                            field.ty,
                            substitutions,
                            item_stack,
                            allow_open_type_parameters,
                        )?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => self.lower_record_row_transform(
                transform,
                *source,
                substitutions,
                item_stack,
                allow_open_type_parameters,
            ),
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_type(
                    *parameter,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
                result: Box::new(self.lower_type(
                    *result,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                self.lower_type_application(
                    *callee,
                    &lowered_arguments,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )
            }
        }
    }

    pub(crate) fn lower_record_row_transform(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let source = self.lower_type(
            source,
            substitutions,
            item_stack,
            allow_open_type_parameters,
        )?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn apply_record_row_transform(
        &self,
        transform: &crate::RecordRowTransform,
        source: &GateType,
    ) -> Option<GateType> {
        let GateType::Record(fields) = source else {
            return None;
        };
        let field_index = fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.name.as_str(), index))
            .collect::<HashMap<_, _>>();
        match transform {
            crate::RecordRowTransform::Pick(labels) => labels
                .iter()
                .map(|label| fields.get(*field_index.get(label.text())?).cloned())
                .collect::<Option<Vec<_>>>()
                .map(GateType::Record),
            crate::RecordRowTransform::Omit(labels) => {
                let omitted = labels
                    .iter()
                    .map(|label| field_index.get(label.text()).copied())
                    .collect::<Option<HashSet<_>>>()?;
                Some(GateType::Record(
                    fields
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !omitted.contains(index))
                        .map(|(_, field)| field.clone())
                        .collect(),
                ))
            }
            crate::RecordRowTransform::Optional(labels)
            | crate::RecordRowTransform::Defaulted(labels) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        if labels.iter().any(|label| label.text() == field.name) {
                            GateRecordField {
                                name: field.name.clone(),
                                ty: match &field.ty {
                                    GateType::Option(_) => field.ty.clone(),
                                    other => GateType::Option(Box::new(other.clone())),
                                },
                            }
                        } else {
                            field.clone()
                        }
                    })
                    .collect(),
            )),
            crate::RecordRowTransform::Required(labels) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        if labels.iter().any(|label| label.text() == field.name) {
                            GateRecordField {
                                name: field.name.clone(),
                                ty: match &field.ty {
                                    GateType::Option(inner) => inner.as_ref().clone(),
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
                    .collect::<Option<HashMap<_, _>>>()?;
                let mut result = Vec::with_capacity(fields.len());
                let mut seen = HashSet::with_capacity(fields.len());
                for (index, field) in fields.iter().enumerate() {
                    let name = renamed
                        .get(&index)
                        .map(|rename| rename.to.text().to_owned())
                        .unwrap_or_else(|| field.name.clone());
                    if !seen.insert(name.clone()) {
                        return None;
                    }
                    result.push(GateRecordField {
                        name,
                        ty: field.ty.clone(),
                    });
                }
                Some(GateType::Record(result))
            }
        }
    }

    pub(crate) fn lower_type_reference(
        &mut self,
        reference: &TypeReference,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => None,
            ResolutionState::Resolved(TypeResolution::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            )) => Some(GateType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::Builtin(_)) => None,
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned().or_else(|| {
                    allow_open_type_parameters.then(|| GateType::TypeParameter {
                        parameter: *parameter,
                        name: self.module.type_parameters()[*parameter]
                            .name
                            .text()
                            .to_owned(),
                    })
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, allow_open_type_parameters)
            }
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                let name = self.module.imports()[*import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.import_type_for_domain_or_opaque(*import_id, name, Vec::new()))
            }
        }
    }

    pub(crate) fn lower_type_application(
        &mut self,
        callee: TypeId,
        arguments: &[GateType],
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Some(GateType::List(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                Some(GateType::Map {
                    key: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                Some(GateType::Set(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                Some(GateType::Option(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                Some(GateType::Result {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                Some(GateType::Validation {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                Some(GateType::Signal(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                Some(GateType::Task {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, arguments, item_stack, allow_open_type_parameters)
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned()
            }
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                let name = self.module.imports()[*import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.import_type_for_domain_or_opaque(*import_id, name, arguments.to_vec()))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(
                BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes,
            ))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn lower_type_item(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let item = &self.module.items()[item_id];
        let name = item_type_name(item);
        if item_stack.contains(&item_id) {
            return Some(GateType::OpaqueItem {
                item: item_id,
                name,
                arguments: arguments.to_vec(),
            });
        }
        item_stack.push(item_id);
        let lowered = match item {
            Item::Type(item) => {
                if item.parameters.len() != arguments.len() {
                    None
                } else {
                    match &item.body {
                        crate::hir::TypeItemBody::Alias(alias) => {
                            let substitutions = item
                                .parameters
                                .iter()
                                .copied()
                                .zip(arguments.iter().cloned())
                                .collect::<HashMap<_, _>>();
                            self.lower_type(
                                *alias,
                                &substitutions,
                                item_stack,
                                allow_open_type_parameters,
                            )
                        }
                        crate::hir::TypeItemBody::Sum(_) => Some(GateType::OpaqueItem {
                            item: item_id,
                            name: item.name.text().to_owned(),
                            arguments: arguments.to_vec(),
                        }),
                    }
                }
            }
            Item::Domain(item) => Some(GateType::Domain {
                item: item_id,
                name: item.name.text().to_owned(),
                arguments: arguments.to_vec(),
            }),
            Item::Class(_)
            | Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        let popped = item_stack.pop();
        debug_assert_eq!(popped, Some(item_id));
        lowered
    }

    pub(crate) fn lower_poly_type(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => {
                self.lower_poly_type_reference(reference, bindings, item_stack)
            }
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => {
                self.lower_poly_record_row_transform(transform, *source, bindings, item_stack)
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_poly_type(*parameter, bindings, item_stack)?),
                result: Box::new(self.lower_poly_type(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        self.apply_type_constructor(binding.head, &all_arguments, item_stack)
                    }
                    _ => {
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        (lowered_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(head, &lowered_arguments, item_stack)
                            })
                            .flatten()
                    }
                }
            }
        }
    }

    pub(crate) fn lower_poly_record_row_transform(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let source = self.lower_poly_type(source, bindings, item_stack)?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn lower_poly_record_row_transform_partially(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let source = self.lower_poly_type_partially(source, bindings, item_stack)?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn lower_poly_type_reference(
        &mut self,
        reference: &TypeReference,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                match bindings.get(parameter)? {
                    TypeBinding::Type(ty) => Some(ty.clone()),
                    TypeBinding::Constructor(binding) => {
                        self.apply_type_constructor(binding.head, &binding.arguments, item_stack)
                    }
                }
            }
            ResolutionState::Resolved(TypeResolution::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            )) => Some(GateType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, false)
            }
            ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn instantiate_poly_type_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<TypeBinding> {
        let mut item_stack = Vec::new();
        if let Some(ty) = self.lower_poly_type(type_id, bindings, &mut item_stack) {
            return Some(TypeBinding::Type(ty));
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    bindings.get(parameter).cloned()
                }
                _ => self
                    .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                    .map(TypeBinding::Constructor),
            },
            TypeKind::Apply { .. } => self
                .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                .map(TypeBinding::Constructor),
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn partial_poly_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                        return None;
                    };
                    Some(binding.clone())
                }
                _ => {
                    let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                    (arity > 0).then_some(TypeConstructorBinding {
                        head,
                        arguments: Vec::new(),
                    })
                }
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() < arity).then_some(TypeConstructorBinding {
                            head: binding.head,
                            arguments: all_arguments,
                        })
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() >= arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        Some(TypeConstructorBinding {
                            head,
                            arguments: lowered_arguments,
                        })
                    }
                }
            }
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn match_poly_hir_type_inner(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let candidate = TypeBinding::Type(actual.clone());
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(&candidate),
                        Entry::Vacant(entry) => {
                            entry.insert(candidate);
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_poly_hir_type_inner(*element, actual, bindings, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_poly_hir_type_inner(
                            field.ty,
                            &actual_field.ty,
                            bindings,
                            item_stack,
                        )
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_poly_hir_type_inner(parameter, actual_parameter, bindings, item_stack)
                    && self.match_poly_hir_type_inner(result, actual_result, bindings, item_stack)
            }
            TypeKind::Apply { callee, arguments } => {
                self.match_poly_type_application(callee, &arguments, actual, bindings, item_stack)
            }
            TypeKind::RecordTransform { .. } => false,
        }
    }

    pub(crate) fn match_poly_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                let Some((head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                let pattern_arguments = arguments.iter().copied().collect::<Vec<_>>();
                if actual_arguments.len() < pattern_arguments.len() {
                    return false;
                }
                let prefix_count = actual_arguments.len() - pattern_arguments.len();
                let candidate = TypeBinding::Constructor(TypeConstructorBinding {
                    head,
                    arguments: actual_arguments[..prefix_count].to_vec(),
                });
                match bindings.entry(*parameter) {
                    Entry::Occupied(entry) if !entry.get().matches(&candidate) => return false,
                    Entry::Occupied(_) => {}
                    Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                }
                pattern_arguments
                    .iter()
                    .zip(actual_arguments[prefix_count..].iter())
                    .all(|(argument, actual_argument)| {
                        self.match_poly_hir_type_inner(
                            *argument,
                            actual_argument,
                            bindings,
                            item_stack,
                        )
                    })
            }
            _ => {
                let Some((expected_head, _)) = self.type_constructor_head_and_arity(callee) else {
                    return false;
                };
                let Some((actual_head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                let heads_match = expected_head == actual_head
                    || self.constructor_heads_same_name(&expected_head, actual);
                heads_match
                    && actual_arguments.len() >= arguments.len()
                    && arguments.iter().zip(actual_arguments.iter()).all(
                        |(argument, actual_argument)| {
                            self.match_poly_hir_type_inner(
                                *argument,
                                actual_argument,
                                bindings,
                                item_stack,
                            )
                        },
                    )
            }
        }
    }

    pub(crate) fn partial_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(_) => {
                let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                (arity > 0).then_some(TypeConstructorBinding {
                    head,
                    arguments: Vec::new(),
                })
            }
            TypeKind::Apply { callee, arguments } => {
                let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                if arguments.len() >= arity {
                    return None;
                }
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        &HashMap::new(),
                        item_stack,
                        false,
                    )?);
                }
                Some(TypeConstructorBinding {
                    head,
                    arguments: lowered_arguments,
                })
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
            TypeKind::RecordTransform { .. } => None,
        }
    }

    pub(crate) fn type_constructor_head_and_arity(
        &self,
        type_id: TypeId,
    ) -> Option<(TypeConstructorHead, usize)> {
        let TypeKind::Name(reference) = &self.module.types()[type_id].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => Some((
                TypeConstructorHead::Builtin(*builtin),
                builtin_type_arity(*builtin),
            )),
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                let arity = match &self.module.items()[*item_id] {
                    Item::Type(item) => item.parameters.len(),
                    Item::Domain(item) => item.parameters.len(),
                    _ => return None,
                };
                Some((TypeConstructorHead::Item(*item_id), arity))
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn apply_type_constructor(
        &mut self,
        head: TypeConstructorHead,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match head {
            TypeConstructorHead::Builtin(builtin) => {
                self.apply_builtin_type_constructor(builtin, arguments)
            }
            TypeConstructorHead::Item(item_id) => {
                self.lower_type_item(item_id, arguments, item_stack, false)
            }
            TypeConstructorHead::Import(import_id) => {
                let name = self.module.imports()[import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.opaque_import_type(import_id, name, arguments.to_vec()))
            }
        }
    }

    /// Check whether an expected TypeConstructorHead matches an actual GateType
    /// by canonical name, handling cross-variant cases (Item vs Import).
    fn constructor_heads_same_name(
        &self,
        expected: &TypeConstructorHead,
        actual: &GateType,
    ) -> bool {
        let expected_name = match expected {
            TypeConstructorHead::Item(item_id) => {
                Some(item_type_name(&self.module.items()[*item_id]))
            }
            TypeConstructorHead::Import(import_id) => self
                .module
                .imports()
                .get(*import_id)
                .map(|imp| imp.imported_name.text().to_owned()),
            TypeConstructorHead::Builtin(_) => None,
        };
        let actual_name = actual.named_type_parts().map(|(n, _)| n);
        match (expected_name, actual_name) {
            (Some(en), Some(an)) => en == an,
            _ => false,
        }
    }

    pub(crate) fn apply_builtin_type_constructor(
        &self,
        builtin: BuiltinType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        if arguments.len() != builtin_type_arity(builtin) {
            return None;
        }
        match builtin {
            BuiltinType::Int
            | BuiltinType::Float
            | BuiltinType::Decimal
            | BuiltinType::BigInt
            | BuiltinType::Bool
            | BuiltinType::Text
            | BuiltinType::Unit
            | BuiltinType::Bytes => Some(GateType::Primitive(builtin)),
            BuiltinType::List => Some(GateType::List(Box::new(arguments.first()?.clone()))),
            BuiltinType::Map => Some(GateType::Map {
                key: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Set => Some(GateType::Set(Box::new(arguments.first()?.clone()))),
            BuiltinType::Option => Some(GateType::Option(Box::new(arguments.first()?.clone()))),
            BuiltinType::Result => Some(GateType::Result {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Validation => Some(GateType::Validation {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Signal => Some(GateType::Signal(Box::new(arguments.first()?.clone()))),
            BuiltinType::Task => Some(GateType::Task {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
        }
    }

    pub(crate) fn infer_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> GateExprInfo {
        let expr = self.module.exprs()[expr_id].clone();
        let info = match expr.kind {
            ExprKind::Name(reference) => self.infer_name(&reference, env),
            ExprKind::Integer(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Int)),
                ..GateExprInfo::default()
            },
            ExprKind::Float(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Float)),
                ..GateExprInfo::default()
            },
            ExprKind::Decimal(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Decimal)),
                ..GateExprInfo::default()
            },
            ExprKind::BigInt(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::BigInt)),
                ..GateExprInfo::default()
            },
            ExprKind::SuffixedInteger(literal) => match self
                .select_suffixed_integer_candidate(&literal, None)
            {
                LiteralSuffixSelection::Unique { result, .. } => GateExprInfo {
                    ty: Some(result),
                    ..GateExprInfo::default()
                },
                LiteralSuffixSelection::Ambiguous { candidates } => GateExprInfo {
                    issues: vec![GateIssue::AmbiguousLiteralSuffix {
                        span: literal.suffix.span(),
                        suffix: literal.suffix.text().to_owned(),
                        candidates,
                    }],
                    ..GateExprInfo::default()
                },
                LiteralSuffixSelection::NoMatch { candidates } => {
                    if candidates.is_empty() {
                        GateExprInfo {
                            issues: vec![GateIssue::UnknownLiteralSuffix {
                                span: literal.suffix.span(),
                                suffix: literal.suffix.text().to_owned(),
                            }],
                            ..GateExprInfo::default()
                        }
                    } else {
                        GateExprInfo::default()
                    }
                }
            },
            ExprKind::Text(text) => {
                let mut info = GateExprInfo {
                    ty: Some(GateType::Primitive(BuiltinType::Text)),
                    ..GateExprInfo::default()
                };
                for segment in text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        info.merge(self.infer_expr(interpolation.expr, env, ambient));
                    }
                }
                info
            }
            ExprKind::Regex(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Text)),
                ..GateExprInfo::default()
            },
            ExprKind::Tuple(elements) => {
                let mut info = GateExprInfo::default();
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    let child = self.infer_expr(*element, env, ambient);
                    if let Some(ty) = child.actual() {
                        lowered.push(ty);
                    }
                    info.merge(child);
                }
                if lowered.len() == elements.len() {
                    info.set_actual(SourceOptionActualType::Tuple(lowered));
                }
                info
            }
            ExprKind::List(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut element_gate_type = None::<GateType>;
                let mut consistent = true;
                for element in &elements {
                    let child = self.infer_expr(*element, env, ambient);
                    if consistent
                        && let Some(child_ty) = child.actual_gate_type().or(child.ty.clone()) {
                            element_gate_type = match element_gate_type.take() {
                                None => Some(child_ty),
                                Some(current) => {
                                    if current.same_shape(&child_ty) {
                                        Some(current)
                                    } else {
                                        consistent = false;
                                        None
                                    }
                                }
                            };
                        }
                    if consistent {
                        if let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    } else {
                        let _ = child.actual();
                    }
                    info.merge(child);
                }
                if consistent {
                    if let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::List(Box::new(element_type)));
                        if info.ty.is_none()
                            && let Some(element_gate_type) = element_gate_type {
                                info.ty = Some(GateType::List(Box::new(element_gate_type)));
                            }
                    } else if let Some(element_gate_type) = element_gate_type {
                        info.ty = Some(GateType::List(Box::new(element_gate_type)));
                    }
                }
                info
            }
            ExprKind::Map(map) => {
                let mut info = GateExprInfo::default();
                let mut key_type = None::<SourceOptionActualType>;
                let mut value_type = None::<SourceOptionActualType>;
                let mut keys_consistent = true;
                let mut values_consistent = true;
                for entry in &map.entries {
                    let key = self.infer_expr(entry.key, env, ambient);
                    if keys_consistent
                        && let Some(child_ty) = key.actual() {
                            key_type = match key_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        keys_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    info.merge(key);

                    let value = self.infer_expr(entry.value, env, ambient);
                    if values_consistent
                        && let Some(child_ty) = value.actual() {
                            value_type = match value_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        values_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    info.merge(value);
                }
                if keys_consistent && values_consistent
                    && let (Some(key), Some(value)) = (key_type, value_type) {
                        info.set_actual(SourceOptionActualType::Map {
                            key: Box::new(key),
                            value: Box::new(value),
                        });
                    }
                info
            }
            ExprKind::Set(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut consistent = true;
                for element in elements {
                    let child = self.infer_expr(element, env, ambient);
                    if consistent
                        && let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    info.merge(child);
                }
                if consistent
                    && let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::Set(Box::new(element_type)));
                    }
                info
            }
            ExprKind::Lambda(_) => GateExprInfo::default(),
            ExprKind::Record(record) => {
                let mut info = GateExprInfo::default();
                let field_count = record.fields.len();
                let mut fields = Vec::with_capacity(field_count);
                for field in record.fields {
                    let child = self.infer_expr(field.value, env, ambient);
                    if let Some(ty) = child.actual() {
                        fields.push(SourceOptionActualRecordField {
                            name: field.label.text().to_owned(),
                            ty,
                        });
                    }
                    info.merge(child);
                }
                if fields.len() == field_count {
                    info.set_actual(SourceOptionActualType::Record(fields));
                }
                info
            }
            ExprKind::Projection { base, path } => {
                let mut info = GateExprInfo::default();
                let subject = match base {
                    crate::hir::ProjectionBase::Ambient => ambient.cloned(),
                    crate::hir::ProjectionBase::Expr(base) => {
                        let base_info = self.infer_expr(base, env, ambient);
                        let ty = base_info.ty.clone();
                        info.merge(base_info);
                        ty
                    }
                };
                if let Some(subject) = subject {
                    match self.project_type(&subject, &path, env.current_domain) {
                        Ok(projected) => info.ty = Some(projected),
                        Err(issue) => info.issues.push(issue),
                    }
                } else {
                    info.issues.push(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(&path),
                        subject: "unknown subject".to_owned(),
                    });
                }
                info
            }
            ExprKind::AmbientSubject => {
                let mut info = GateExprInfo::default();
                if let Some(ambient) = ambient.cloned() {
                    info.ty = Some(ambient);
                } else {
                    info.issues
                        .push(GateIssue::AmbientSubjectOutsidePipe { span: expr.span });
                }
                info
            }
            ExprKind::Apply { callee, arguments } => {
                let mut same_module_function_item = None;
                if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind {
                    if let Some(info) = self
                        .infer_builtin_constructor_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let Some(info) =
                        self.infer_domain_constructor_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let Some(info) =
                        self.infer_domain_member_apply(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let Some(info) =
                        self.infer_class_member_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let Some(info) = self.infer_same_module_constructor_apply_expr(
                        reference, &arguments, env, ambient,
                    ) {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let ResolutionState::Resolved(TermResolution::Item(item_id)) =
                        reference.resolution.as_ref()
                        && let Item::Function(function) = &self.module.items()[*item_id]
                        && supports_same_module_function_inference(function)
                    {
                        same_module_function_item = Some(*item_id);
                    }
                    if let Some(info) = self
                        .infer_polymorphic_function_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                    if let Some(info) =
                        self.infer_import_function_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(
                            self.maybe_use_ambient_signal_payload(expr_id, ambient, info),
                        );
                    }
                }
                let mut info = self.infer_expr(callee, env, ambient);
                let mut current = info.ty.clone();
                for argument in arguments.iter() {
                    // Extract the expected parameter type from the current Arrow type.
                    // This lets constructors like `None` / `Some` / `Ok` resolve when
                    // the callee's parameter type is known, mirroring the import path.
                    let (param_ty, fallback_result) = match &current {
                        Some(GateType::Arrow { parameter, result }) => (
                            Some(parameter.as_ref().clone()),
                            Some(result.as_ref().clone()),
                        ),
                        _ => (None, None),
                    };
                    let argument_info =
                        self.infer_expr(*argument, env, param_ty.as_ref().or(ambient));
                    // If inference returns no type, fall back to the parameter type so the
                    // Arrow chain can still be advanced (same strategy as import path).
                    let argument_ty = argument_info
                        .actual_gate_type()
                        .or(argument_info.ty.clone())
                        .or_else(|| param_ty.clone());
                    info.merge(argument_info);
                    current = match (current.as_ref(), argument_ty.as_ref()) {
                        (Some(callee_ty), Some(argument_ty)) => self
                            .apply_function(callee_ty, argument_ty)
                            .or(fallback_result),
                        _ => fallback_result,
                    };
                }
                if let Some(item_id) = same_module_function_item
                    && let Some(argument_types) = arguments
                        .iter()
                        .map(|argument| {
                            let argument_info = self.infer_expr(*argument, env, None);
                            argument_info.actual_gate_type().or(argument_info.ty)
                        })
                        .collect::<Option<Vec<_>>>()
                    && argument_types
                        .iter()
                        .all(|argument_ty| !argument_ty.has_type_params())
                    && current
                        .as_ref()
                        .is_none_or(|result_ty| !result_ty.has_type_params())
                {
                    self.record_function_call_evidence(FunctionCallEvidence {
                        item_id,
                        argument_types,
                        result_type: current.clone(),
                    });
                }
                info.ty = current;
                info
            }
            ExprKind::Unary { operator, expr } => {
                let mut info = self.infer_expr(expr, env, ambient);
                let operand_ty = info.actual_gate_type().or(info.ty.clone());
                info.ty = match (operator, operand_ty.as_ref()) {
                    (crate::hir::UnaryOperator::Not, Some(ty)) if ty.is_bool() => {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let mut left_info = self.infer_expr(left, env, ambient);
                let mut left_ty = left_info.actual_gate_type().or(left_info.ty.clone());
                let mut right_info = self.infer_expr(right, env, ambient);
                let mut right_ty = right_info.actual_gate_type().or(right_info.ty.clone());

                if right_ty.is_none()
                    && let Some(left) = left_ty.as_ref()
                    && let Some(refined) =
                        self.infer_binary_operand_against_peer(right, env, ambient, left)
                {
                    right_ty = refined.actual_gate_type().or(refined.ty.clone());
                    right_info = refined;
                }
                if left_ty.is_none()
                    && let Some(right) = right_ty.as_ref()
                    && let Some(refined) =
                        self.infer_binary_operand_against_peer(left, env, ambient, right)
                {
                    left_ty = refined.actual_gate_type().or(refined.ty.clone());
                    left_info = refined;
                }

                let mut info = left_info;
                info.merge(right_info);
                info.ty = if let (Some(left), Some(right)) = (left_ty.as_ref(), right_ty.as_ref()) {
                    match select_domain_binary_operator(self.module, self, operator, left, right) {
                        Ok(maybe_matched) => maybe_matched.map(|matched| matched.result_type),
                        Err(candidates) => {
                            // Multiple domain operator implementations match: emit an ambiguity
                            // diagnostic and leave the result type unknown so downstream checking
                            // can continue without cascading false errors.
                            info.issues.push(GateIssue::AmbiguousDomainOperator {
                                span: expr.span,
                                operator: binary_operator_text(operator).to_owned(),
                                candidates: candidates
                                    .into_iter()
                                    .map(|c| {
                                        format!("{}.{}", c.callee.domain_name, c.callee.member_name)
                                    })
                                    .collect(),
                            });
                            None
                        }
                    }
                } else {
                    None
                };
                if info.ty.is_some() {
                    return self.finalize_expr_info(info);
                }
                info.ty = match (left_ty.as_ref(), right_ty.as_ref(), operator) {
                    (Some(left), Some(right), crate::hir::BinaryOperator::And)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Or)
                        if left.is_bool() && right.is_bool() =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::GreaterThan)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::LessThan)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::GreaterThanOrEqual)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::LessThanOrEqual)
                        if left.same_shape(right)
                            && crate::typecheck::resolve_ordering_dispatch(self.module, left)
                                .is_some() =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Add)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Subtract)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Multiply)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Divide)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Modulo)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(left.clone())
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Equals)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::NotEquals)
                        if left.same_shape(right) =>
                    {
                        info.constraints
                            .push(TypeConstraint::eq(expr.span, left.clone()));
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Pipe(pipe) => self.infer_pipe_expr(&pipe, env),
            ExprKind::Cluster(cluster) => self.infer_cluster_expr(cluster, env),
            ExprKind::PatchApply { target, patch } => {
                let mut info = self.infer_expr(target, env, ambient);
                info.actual = info
                    .actual
                    .clone()
                    .or_else(|| info.ty.as_ref().map(SourceOptionActualType::from_gate_type));
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            info.merge(self.infer_expr(*expr, env, ambient));
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            info.merge(self.infer_expr(expr, env, ambient));
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                info
            }
            ExprKind::PatchLiteral(patch) => {
                let mut info = GateExprInfo::default();
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            info.merge(self.infer_expr(*expr, env, ambient));
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            info.merge(self.infer_expr(expr, env, ambient));
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                info
            }
            ExprKind::Markup(_) => GateExprInfo::default(),
        };
        self.finalize_expr_info(self.maybe_use_ambient_signal_payload(expr_id, ambient, info))
    }

    pub(crate) fn infer_expr_with_expected(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        expected: &GateType,
    ) -> GateExprInfo {
        let expr = self.module.exprs()[expr_id].clone();
        match expr.kind {
            ExprKind::SuffixedInteger(literal) => {
                self.infer_suffixed_integer_expr_with_expected(&literal, expected)
            }
            ExprKind::Name(reference) => self.infer_name_with_expected(&reference, env, expected),
            _ => self.infer_expr(expr_id, env, ambient),
        }
    }

    fn infer_binary_operand_against_peer(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        peer: &GateType,
    ) -> Option<GateExprInfo> {
        let expr = self.module.exprs()[expr_id].clone();
        matches!(expr.kind, ExprKind::SuffixedInteger(_))
            .then(|| self.infer_expr_with_expected(expr_id, env, ambient, peer))
    }

    fn infer_suffixed_integer_expr_with_expected(
        &mut self,
        literal: &crate::hir::SuffixedIntegerLiteral,
        expected: &GateType,
    ) -> GateExprInfo {
        match self.select_suffixed_integer_candidate(literal, Some(expected)) {
            LiteralSuffixSelection::Unique { result, .. } => GateExprInfo {
                ty: Some(result),
                ..GateExprInfo::default()
            },
            LiteralSuffixSelection::Ambiguous { candidates } => GateExprInfo {
                issues: vec![GateIssue::AmbiguousLiteralSuffix {
                    span: literal.suffix.span(),
                    suffix: literal.suffix.text().to_owned(),
                    candidates,
                }],
                ..GateExprInfo::default()
            },
            LiteralSuffixSelection::NoMatch { candidates } => {
                if candidates.is_empty() {
                    GateExprInfo {
                        issues: vec![GateIssue::UnknownLiteralSuffix {
                            span: literal.suffix.span(),
                            suffix: literal.suffix.text().to_owned(),
                        }],
                        ..GateExprInfo::default()
                    }
                } else {
                    GateExprInfo::default()
                }
            }
        }
    }

    pub(crate) fn infer_name(
        &mut self,
        reference: &TermReference,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => GateExprInfo::default(),
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                let ty = env.locals.get(binding).cloned();
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                let constructor_ty = self.infer_same_module_constructor_name_type(reference);
                let ty = constructor_ty
                    .clone()
                    .or_else(|| self.item_value_type(*item_id));
                let actual = constructor_ty
                    .as_ref()
                    .map(SourceOptionActualType::from_gate_type)
                    .or_else(|| self.item_value_actual(*item_id));
                GateExprInfo {
                    actual,
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                let ty = self.import_value_type(*import_id);
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::DomainConstructor(item_id)) => GateExprInfo {
                ty: self.infer_domain_constructor_name_type(*item_id),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::DomainMember(_)) => GateExprInfo {
                ty: self.infer_domain_member_name_type(reference),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_)) => GateExprInfo {
                issues: vec![GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                }],
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => {
                GateExprInfo::default()
            }
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(candidates)) => {
                // Without a known expected type, try to provide a type hint if all
                // candidates share the same type shape (e.g. all are `List A -> Bool`
                // with different concrete `A`s — uncommon but helpful for error messages).
                // Disambiguation with actual expected type happens in runtime_reference_for_name.
                let types: Vec<_> = candidates
                    .iter()
                    .filter_map(|id| self.import_value_type(*id))
                    .collect();
                let common_ty = if types.len() == candidates.len()
                    && types.windows(2).all(|w| w[0].same_shape(&w[1]))
                {
                    types.into_iter().next()
                } else {
                    None
                };
                GateExprInfo {
                    contains_signal: common_ty.as_ref().is_some_and(GateType::is_signal),
                    ty: common_ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(value)) => GateExprInfo {
                ty: Some(self.intrinsic_value_type(*value)),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                let (ty, actual) = match builtin {
                    crate::hir::BuiltinTerm::True | crate::hir::BuiltinTerm::False => {
                        (Some(GateType::Primitive(BuiltinType::Bool)), None)
                    }
                    crate::hir::BuiltinTerm::None => (
                        None,
                        Some(SourceOptionActualType::Option(Box::new(
                            SourceOptionActualType::Hole,
                        ))),
                    ),
                    crate::hir::BuiltinTerm::Some
                    | crate::hir::BuiltinTerm::Ok
                    | crate::hir::BuiltinTerm::Err
                    | crate::hir::BuiltinTerm::Valid
                    | crate::hir::BuiltinTerm::Invalid => (None, None),
                };
                GateExprInfo {
                    actual,
                    ty,
                    ..GateExprInfo::default()
                }
            }
        }
    }

    pub(crate) fn infer_name_with_expected(
        &mut self,
        reference: &TermReference,
        env: &GateExprEnv,
        expected: &GateType,
    ) -> GateExprInfo {
        let mut info = match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                if let Some(ty) = self.match_item_name_candidate(*item_id, expected) {
                    GateExprInfo {
                        actual: Some(SourceOptionActualType::from_gate_type(&ty)),
                        contains_signal: ty.is_signal(),
                        ty: Some(ty),
                        ..GateExprInfo::default()
                    }
                } else {
                    self.infer_name(reference, env)
                }
            }
            ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_)) => {
                match self.select_domain_member_name(reference, expected) {
                    Some(DomainMemberSelection::Unique(ty)) => GateExprInfo {
                        actual: Some(SourceOptionActualType::from_gate_type(&ty)),
                        contains_signal: ty.is_signal(),
                        ty: Some(ty),
                        ..GateExprInfo::default()
                    },
                    Some(DomainMemberSelection::Ambiguous) => GateExprInfo {
                        issues: vec![GateIssue::AmbiguousDomainMember {
                            span: reference.span(),
                            name: reference.path.segments().last().text().to_owned(),
                            candidates: self
                                .domain_member_candidate_labels(reference)
                                .unwrap_or_default(),
                        }],
                        ..GateExprInfo::default()
                    },
                    Some(DomainMemberSelection::NoMatch) | None => self.infer_name(reference, env),
                }
            }
            _ => self.infer_name(reference, env),
        };
        if let Some(specialized) = info
            .ty
            .as_ref()
            .and_then(|template| self.specialize_gate_type_template(template, expected))
        {
            info.actual = Some(SourceOptionActualType::from_gate_type(&specialized));
            info.contains_signal = specialized.is_signal();
            info.ty = Some(specialized);
        }
        info
    }

    pub(crate) fn same_module_constructor(
        &self,
        reference: &TermReference,
    ) -> Option<(ItemId, String, Vec<TypeParameterId>, Vec<TypeVariantField>)> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Type(item) = &self.module.items()[*item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let variant_name = reference.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        Some((
            *item_id,
            item.name.text().to_owned(),
            item.parameters.clone(),
            variant.fields.clone(),
        ))
    }

    fn domain_constructor_type(
        &self,
        item_id: ItemId,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        let Item::Domain(domain) = &self.module.items()[item_id] else {
            return None;
        };
        let arguments = domain
            .parameters
            .iter()
            .map(|parameter| substitutions.get(parameter).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(GateType::Domain {
            item: item_id,
            name: domain.name.text().to_owned(),
            arguments,
        })
    }

    pub(crate) fn infer_domain_constructor_name_type(
        &mut self,
        item_id: ItemId,
    ) -> Option<GateType> {
        let Item::Domain(domain) = &self.module.items()[item_id] else {
            return None;
        };
        if !domain.parameters.is_empty() {
            return None;
        }
        let carrier = self.lower_open_annotation(domain.carrier)?;
        let result = self.domain_constructor_type(item_id, &HashMap::new())?;
        Some(GateType::Arrow {
            parameter: Box::new(carrier),
            result: Box::new(result),
        })
    }

    pub(crate) fn infer_domain_constructor_apply(
        &mut self,
        item_id: ItemId,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<GateType> {
        let [argument_ty] = argument_types else {
            return None;
        };
        let Item::Domain(domain) = &self.module.items()[item_id] else {
            return None;
        };
        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !self.match_hir_type(domain.carrier, argument_ty, &mut substitutions, &mut item_stack) {
            return None;
        }
        let result = self.domain_constructor_type(item_id, &substitutions)?;
        if let Some(expected) = expected_result
            && !result.same_shape(expected)
        {
            return None;
        }
        Some(result)
    }

    pub(crate) fn infer_domain_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::DomainConstructor(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        info.ty = self.infer_domain_constructor_apply(*item_id, &argument_types, None);
        Some(info)
    }

    fn literal_suffix_resolution_label(
        &self,
        resolution: LiteralSuffixResolution,
    ) -> Option<String> {
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                let member = domain.members.get(resolution.member_index)?;
                Some(format!("{}.{}", domain.name.text(), member.name.text()))
            }
            LiteralSuffixResolution::Import(import_id) => {
                let import = self.module.imports().get(import_id)?;
                let ImportBindingMetadata::DomainSuffix {
                    domain_name,
                    suffix_name,
                    ..
                } = &import.metadata
                else {
                    return None;
                };
                Some(format!("{domain_name}.{suffix_name}"))
            }
        }
    }

    fn visible_integer_literal_suffix_candidates(
        &self,
        suffix: &Name,
    ) -> Vec<LiteralSuffixResolution> {
        let local = self
            .module
            .root_items()
            .iter()
            .filter_map(|item_id| match &self.module.items()[*item_id] {
                Item::Domain(domain) => Some(
                    domain
                        .members
                        .iter()
                        .enumerate()
                        .filter(|(_, member)| {
                            member.kind == DomainMemberKind::Literal && member.name.text() == suffix.text()
                        })
                        .map(|(member_index, _)| {
                            LiteralSuffixResolution::DomainMember(DomainMemberResolution {
                                domain: *item_id,
                                member_index,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        if !local.is_empty() {
            return local;
        }

        let imported = self
            .module
            .imports()
            .iter()
            .filter_map(|(import_id, import)| match &import.metadata {
                ImportBindingMetadata::DomainSuffix { suffix_name, .. }
                    if suffix_name.as_ref() == suffix.text() =>
                {
                    Some(LiteralSuffixResolution::Import(import_id))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if !imported.is_empty() {
            return imported;
        }

        self.module
            .ambient_items()
            .iter()
            .filter_map(|item_id| match &self.module.items()[*item_id] {
                Item::Domain(domain) => Some(
                    domain
                        .members
                        .iter()
                        .enumerate()
                        .filter(|(_, member)| {
                            member.kind == DomainMemberKind::Literal && member.name.text() == suffix.text()
                        })
                        .map(|(member_index, _)| {
                            LiteralSuffixResolution::DomainMember(DomainMemberResolution {
                                domain: *item_id,
                                member_index,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn literal_suffix_base(
        &self,
        resolution: LiteralSuffixResolution,
    ) -> Option<LiteralSuffixBase> {
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                let member = domain.members.get(resolution.member_index)?;
                let annotation = self.module.types().get(member.annotation)?;
                let parameter = match &annotation.kind {
                    TypeKind::Arrow { parameter, .. } => *parameter,
                    _ => member.annotation,
                };
                match &self.module.types()[parameter].kind {
                    TypeKind::Name(reference) => match reference.resolution.as_ref() {
                        ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Int)) => {
                            Some(LiteralSuffixBase::Int)
                        }
                        ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Decimal)) => {
                            Some(LiteralSuffixBase::Decimal)
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
            LiteralSuffixResolution::Import(import_id) => match &self.module.imports()[import_id].metadata
            {
                ImportBindingMetadata::DomainSuffix { base, .. } => Some(*base),
                _ => None,
            },
        }
    }

    fn literal_suffix_result_type(
        &mut self,
        resolution: LiteralSuffixResolution,
        expected_result: Option<&GateType>,
    ) -> Option<GateType> {
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                let member = domain.members.get(resolution.member_index)?;
                let annotation = self.module.types().get(member.annotation)?;
                let result_type = match &annotation.kind {
                    TypeKind::Arrow { result, .. } => *result,
                    _ => {
                        return Some(GateType::Domain {
                            item: resolution.domain,
                            name: item_type_name(&self.module.items()[resolution.domain]),
                            arguments: Vec::new(),
                        });
                    }
                };
                let mut substitutions = HashMap::new();
                if let Some(expected) = expected_result {
                    let mut item_stack = Vec::new();
                    if !self.match_hir_type(
                        result_type,
                        expected,
                        &mut substitutions,
                        &mut item_stack,
                    ) {
                        return None;
                    }
                }
                self.lower_hir_type(result_type, &substitutions)
            }
            LiteralSuffixResolution::Import(import_id) => {
                let ty = self.import_value_type(import_id)?;
                let result = Self::arrow_result_type(&ty, 1)?;
                if let Some(expected) = expected_result
                    && !result.same_shape(expected)
                {
                    return None;
                }
                Some(result)
            }
        }
    }

    pub(crate) fn select_suffixed_integer_candidate(
        &mut self,
        literal: &crate::hir::SuffixedIntegerLiteral,
        expected_result: Option<&GateType>,
    ) -> LiteralSuffixSelection {
        let visible = self.visible_integer_literal_suffix_candidates(&literal.suffix);
        if visible.is_empty() {
            return LiteralSuffixSelection::NoMatch {
                candidates: Vec::new(),
            };
        }
        let filtered = visible
            .into_iter()
            .filter(|candidate| {
                self.literal_suffix_base(*candidate)
                    .is_some_and(LiteralSuffixBase::accepts_integer_payload)
            })
            .collect::<Vec<_>>();
        if filtered.is_empty() {
            return LiteralSuffixSelection::NoMatch {
                candidates: Vec::new(),
            };
        }
        let mut matches = Vec::new();
        for candidate in filtered.iter().copied() {
            if let Some(result) = self.literal_suffix_result_type(candidate, expected_result) {
                matches.push((candidate, result));
            }
        }
        match matches.len() {
            0 => LiteralSuffixSelection::NoMatch {
                candidates: filtered
                    .into_iter()
                    .filter_map(|candidate| self.literal_suffix_resolution_label(candidate))
                    .collect(),
            },
            1 => {
                let (resolution, result) = matches
                    .pop()
                    .expect("exactly one literal suffix match should be available");
                let base = self
                    .literal_suffix_base(resolution)
                    .expect("matched literal suffix should have a base family");
                LiteralSuffixSelection::Unique {
                    resolution,
                    base,
                    result,
                }
            }
            _ => LiteralSuffixSelection::Ambiguous {
                candidates: matches
                    .into_iter()
                    .filter_map(|(candidate, _)| self.literal_suffix_resolution_label(candidate))
                    .collect(),
            },
        }
    }

    pub(crate) fn lower_suffixed_integer_call(
        &mut self,
        literal: &crate::hir::SuffixedIntegerLiteral,
        expected_result: &GateType,
    ) -> Option<LiteralSuffixCallLowering> {
        let LiteralSuffixSelection::Unique {
            resolution,
            base,
            result,
        } = self.select_suffixed_integer_candidate(literal, Some(expected_result))
        else {
            return None;
        };

        let callee_type = match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                let annotation = domain.members.get(resolution.member_index)?.annotation;
                let TypeKind::Arrow { result, .. } = &self.module.types()[annotation].kind else {
                    return None;
                };
                let mut substitutions = HashMap::new();
                let mut item_stack = Vec::new();
                if !self.match_hir_type(*result, expected_result, &mut substitutions, &mut item_stack)
                {
                    return None;
                }
                let mut lower_stack = Vec::new();
                self.lower_type(annotation, &substitutions, &mut lower_stack, false)?
            }
            LiteralSuffixResolution::Import(import_id) => self.import_value_type(import_id)?,
        };

        Some(LiteralSuffixCallLowering {
            resolution,
            base,
            callee_type,
            result_type: result,
        })
    }

    pub(crate) fn infer_builtin_constructor_actual(
        &self,
        builtin: BuiltinTerm,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        match (builtin, arguments) {
            (BuiltinTerm::True | BuiltinTerm::False, []) => {
                Some(SourceOptionActualType::Primitive(BuiltinType::Bool))
            }
            (BuiltinTerm::None, []) => Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
            (BuiltinTerm::Some, [argument]) => {
                Some(SourceOptionActualType::Option(Box::new(argument.clone())))
            }
            (BuiltinTerm::Ok, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Err, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            (BuiltinTerm::Valid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Invalid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            _ => None,
        }
    }

    pub(crate) fn infer_builtin_constructor_actual_from_reference(
        &self,
        reference: &TermReference,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        self.infer_builtin_constructor_actual(*builtin, arguments)
    }

    pub(crate) fn infer_builtin_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let mut info = GateExprInfo::default();
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            info.merge(argument_info);
            let Some(argument_actual) = argument_actual else {
                return Some(info);
            };
            argument_actuals.push(argument_actual);
        }
        let actual = self.infer_builtin_constructor_actual(*builtin, &argument_actuals)?;
        info.set_actual(actual);
        Some(info)
    }

    pub(crate) fn infer_same_module_constructor_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if !parameters.is_empty() {
            return None;
        }
        let substitutions = HashMap::new();
        let field_types = fields
            .into_iter()
            .map(|field| self.lower_hir_type(field.ty, &substitutions))
            .collect::<Option<Vec<_>>>()?;
        let mut ty = GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments: Vec::new(),
        };
        for field_ty in field_types.into_iter().rev() {
            ty = GateType::Arrow {
                parameter: Box::new(field_ty),
                result: Box::new(ty),
            };
        }
        Some(ty)
    }

    pub(crate) fn infer_same_module_constructor_apply(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_types.len() {
            return None;
        }
        let mut substitutions = HashMap::new();
        for (field, actual) in fields.iter().zip(argument_types.iter()) {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(field.ty, actual, &mut substitutions, &mut item_stack) {
                return None;
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| substitutions.get(parameter).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    pub(crate) fn infer_same_module_constructor_apply_actual(
        &mut self,
        reference: &TermReference,
        argument_actuals: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_actuals.len() {
            return None;
        }
        let validator = Validator {
            module: self.module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
            kind_item_cache: HashMap::new(),
            kind_item_stack: HashSet::new(),
        };
        let mut substitutions = HashMap::<TypeParameterId, SourceOptionActualType>::new();
        for (field, actual) in fields.iter().zip(argument_actuals.iter()) {
            match validator.source_option_hir_type_matches_actual_type_inner(
                field.ty,
                actual,
                &mut substitutions,
            ) {
                Some(true) => {}
                Some(false) | None => return None,
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| {
                substitutions
                    .get(parameter)
                    .cloned()
                    .unwrap_or(SourceOptionActualType::Hole)
            })
            .collect();
        Some(SourceOptionActualType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    pub(crate) fn infer_same_module_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.same_module_constructor(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
            argument_actuals.push(argument_actual);
        }
        if let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() {
            info.ty = self.infer_same_module_constructor_apply(reference, &argument_types);
        }
        if info.ty.is_none() {
            let Some(argument_actuals) = argument_actuals.into_iter().collect::<Option<Vec<_>>>()
            else {
                return Some(info);
            };
            if let Some(actual) =
                self.infer_same_module_constructor_apply_actual(reference, &argument_actuals)
            {
                info.set_actual(actual);
            }
        }
        Some(info)
    }

    pub(crate) fn match_function_signature(
        &mut self,
        function: &crate::hir::FunctionItem,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<(Vec<GateType>, GateType)> {
        if function.parameters.len() < argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(argument_types.len());
        for (parameter, actual) in function.parameters.iter().zip(argument_types.iter()) {
            let annotation = parameter.annotation?;
            if let Some(lowered) = self.lower_annotation(annotation) {
                if !lowered.same_shape(actual) {
                    return None;
                }
                instantiated_parameters.push(lowered);
                continue;
            }
            if !self.match_poly_hir_type(annotation, actual, &mut bindings) {
                return None;
            }
            instantiated_parameters.push(self.instantiate_poly_hir_type(annotation, &bindings)?);
        }
        let result_annotation = function.annotation?;
        if function.parameters.len() == argument_types.len() {
            // Full application: check expected result and return concrete result type.
            if let Some(expected) = expected_result {
                if let Some(lowered) = self.lower_annotation(result_annotation) {
                    if !lowered.same_shape(expected) {
                        return None;
                    }
                } else if !self.match_poly_hir_type(result_annotation, expected, &mut bindings) {
                    return None;
                }
            }
            let result = self
                .lower_annotation(result_annotation)
                .or_else(|| self.instantiate_poly_hir_type(result_annotation, &bindings))?;
            Some((instantiated_parameters, result))
        } else {
            // Partial application: compute curried result type from the remaining parameters
            // and the declared return type, instantiating any bound type parameters.
            let remaining_params = &function.parameters[argument_types.len()..];
            let remaining_types = remaining_params
                .iter()
                .map(|p| {
                    p.annotation.and_then(|ann| {
                        self.lower_annotation(ann)
                            .or_else(|| self.instantiate_poly_hir_type_partially(ann, &bindings))
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            let result_ty = self.lower_annotation(result_annotation).or_else(|| {
                self.instantiate_poly_hir_type_partially(result_annotation, &bindings)
            })?;
            let curried = remaining_types
                .into_iter()
                .rev()
                .fold(result_ty, |acc, param| GateType::Arrow {
                    parameter: Box::new(param),
                    result: Box::new(acc),
                });
            Some((instantiated_parameters, curried))
        }
    }

    pub(crate) fn function_signature(
        &self,
        ty: &GateType,
        arity: usize,
    ) -> Option<(Vec<GateType>, GateType)> {
        let mut parameters = Vec::with_capacity(arity);
        let mut current = ty;
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameters.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some((parameters, current.clone()))
    }

    pub(crate) fn flatten_apply_expr(&self, expr_id: ExprId) -> (ExprId, Vec<ExprId>) {
        let mut callee = expr_id;
        let mut segments = Vec::new();
        while let ExprKind::Apply {
            callee: next_callee,
            arguments: next_arguments,
        } = &self.module.exprs()[callee].kind
        {
            segments.push(next_arguments.iter().copied().collect::<Vec<_>>());
            callee = *next_callee;
        }
        let mut arguments = Vec::new();
        for segment in segments.into_iter().rev() {
            arguments.extend(segment);
        }
        (callee, arguments)
    }

    pub(crate) fn match_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<()> {
        if let Some(lowered) = self.lower_annotation(annotation) {
            return lowered.same_shape(actual).then_some(());
        }
        if let Some(template) = self.lower_open_annotation(annotation) {
            let mut substitutions = HashMap::new();
            Self::match_gate_type_template(&template, actual, &mut substitutions).then_some(())?;
            for (parameter, ty) in substitutions {
                let candidate = TypeBinding::Type(ty);
                match bindings.entry(parameter) {
                    Entry::Occupied(entry) if !entry.get().matches(&candidate) => return None,
                    Entry::Occupied(_) => {}
                    Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                }
            }
            return Some(());
        }
        self.match_poly_hir_type(annotation, actual, bindings)
            .then_some(())
    }

    pub(crate) fn match_pipe_argument_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<bool> {
        if self
            .match_function_parameter_annotation(annotation, actual, bindings)
            .is_some()
        {
            return Some(false);
        }
        let GateType::Signal(payload) = actual else {
            return None;
        };
        self.match_function_parameter_annotation(annotation, payload, bindings)
            .map(|_| true)
    }

    pub(crate) fn instantiate_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_annotation(annotation)
            .or_else(|| self.instantiate_poly_hir_type(annotation, bindings))
            .or_else(|| self.instantiate_poly_hir_type_partially(annotation, bindings))
    }

    pub(crate) fn match_pipe_function_signature(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let (callee_expr, explicit_arguments) = self.flatten_apply_expr(expr_id);
        self.match_pipe_function_signature_parts(
            callee_expr,
            explicit_arguments,
            env,
            ambient,
            expected_result,
        )
    }

    pub(crate) fn match_pipe_function_signature_parts(
        &mut self,
        callee_expr: ExprId,
        explicit_arguments: Vec<ExprId>,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind else {
            return None;
        };
        if let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        {
            let Item::Function(function) = &self.module.items()[*item_id] else {
                return None;
            };
            if function.parameters.len() != explicit_arguments.len() + 1 {
                return None;
            }
            if function.annotation.is_none()
                || function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.annotation.is_none())
            {
                return self.match_pipe_unannotated_function_signature(
                    callee_expr,
                    &explicit_arguments,
                    function,
                    env,
                    ambient,
                    expected_result,
                );
            }

            let mut bindings = PolyTypeBindings::new();
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
                let annotation = parameter.annotation?;
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                let Some(argument_ty) = argument_info.actual_gate_type().or(argument_info.ty)
                else {
                    signal_payload_arguments.push(false);
                    continue;
                };
                let reads_signal_payload = self.match_pipe_argument_parameter_annotation(
                    annotation,
                    &argument_ty,
                    &mut bindings,
                )?;
                signal_payload_arguments.push(reads_signal_payload);
            }

            let ambient_parameter = function
                .parameters
                .last()
                .expect("checked pipe arity above");
            let ambient_annotation = ambient_parameter.annotation?;
            self.match_function_parameter_annotation(ambient_annotation, ambient, &mut bindings)?;

            let result_annotation = function.annotation?;
            if let Some(expected) = expected_result {
                self.match_function_parameter_annotation(
                    result_annotation,
                    expected,
                    &mut bindings,
                )?;
            }

            let mut parameter_types = Vec::with_capacity(function.parameters.len());
            for parameter in &function.parameters {
                let annotation = parameter.annotation?;
                parameter_types
                    .push(self.instantiate_function_parameter_annotation(annotation, &bindings)?);
            }
            let result_type =
                self.instantiate_function_parameter_annotation(result_annotation, &bindings)?;

            return Some(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments,
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }

        // Handle Import and AmbiguousHoistedImports: try to find a unique import candidate
        // by walking its curried Arrow type through explicit args then the ambient subject.
        // This covers hoisted prelude functions like `list.map`, `option.map`, etc.
        let import_candidates: Option<Vec<ImportId>> = match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Import(id)) => Some(vec![*id]),
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(ids)) => {
                Some(ids.iter().copied().collect())
            }
            _ => None,
        };
        if let Some(candidates) = import_candidates {
            return self.match_import_pipe_function_signature(
                callee_expr,
                candidates,
                explicit_arguments,
                env,
                ambient,
                expected_result,
            );
        }

        let explicit_argument_types = explicit_arguments
            .iter()
            .map(|argument| {
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                argument_info.actual_gate_type().or(argument_info.ty)
            })
            .collect::<Vec<_>>();
        if let Some(mut full_argument_types) = explicit_argument_types
            .iter()
            .cloned()
            .collect::<Option<Vec<_>>>()
        {
            full_argument_types.push(ambient.clone());
            if let DomainMemberSelection::Unique(matched) =
                self.select_class_member_call(reference, &full_argument_types, expected_result)?
            {
                return Some(PipeFunctionSignatureMatch {
                    callee_expr,
                    explicit_arguments,
                    signal_payload_arguments: vec![
                        false;
                        matched.parameters.len().saturating_sub(1)
                    ],
                    parameter_types: matched.parameters,
                    result_type: matched.result,
                });
            }
        }
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            let (_, member_annotation, _) = self.class_member_signature(candidate)?;
            let mut bindings = PolyTypeBindings::new();
            let mut current = member_annotation;
            let mut parameter_type_ids = Vec::with_capacity(explicit_arguments.len() + 1);
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for argument_ty in explicit_argument_types.iter() {
                let TypeKind::Arrow { parameter, result } =
                    self.module.types()[current].kind.clone()
                else {
                    continue;
                };
                if let Some(argument_ty) = argument_ty.as_ref() {
                    if self.match_poly_hir_type(parameter, argument_ty, &mut bindings) {
                        signal_payload_arguments.push(false);
                    } else if let GateType::Signal(payload) = argument_ty {
                        if self.match_poly_hir_type(parameter, payload, &mut bindings) {
                            signal_payload_arguments.push(true);
                        } else {
                            parameter_type_ids.clear();
                            signal_payload_arguments.clear();
                            break;
                        }
                    } else {
                        parameter_type_ids.clear();
                        signal_payload_arguments.clear();
                        break;
                    }
                } else {
                    signal_payload_arguments.push(false);
                }
                parameter_type_ids.push(parameter);
                current = result;
            }
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                continue;
            };
            if !self.match_poly_hir_type(parameter, ambient, &mut bindings) {
                continue;
            }
            parameter_type_ids.push(parameter);
            current = result;
            if parameter_type_ids.len() != explicit_arguments.len() + 1 {
                continue;
            }
            if let Some(expected) = expected_result
                && !self.match_poly_hir_type(current, expected, &mut bindings)
            {
                continue;
            }
            let Some(parameter_types) = parameter_type_ids
                .into_iter()
                .map(|parameter| self.instantiate_poly_hir_type_partially(parameter, &bindings))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Some(result_type) = self.instantiate_poly_hir_type_partially(current, &bindings)
            else {
                continue;
            };
            if let Some(expected) = expected_result
                && !result_type.same_shape(expected)
            {
                continue;
            }
            let explicit_arguments_match = explicit_arguments
                .iter()
                .zip(parameter_types.iter().take(explicit_arguments.len()))
                .zip(signal_payload_arguments.iter())
                .all(|((argument, expected_parameter), reads_signal_payload)| {
                    let argument_info = self.infer_expr(*argument, env, Some(ambient));
                    let arg_ty = argument_info
                        .actual_gate_type()
                        .or(argument_info.ty.clone());
                    // If we have no type information for the argument, we can't prove it
                    // doesn't match — accept it and let downstream lowering verify.
                    arg_ty.is_none()
                        || arg_ty.as_ref().is_some_and(|actual| {
                            actual.same_shape(expected_parameter)
                                || (*reads_signal_payload
                                    && matches!(
                                        actual,
                                        GateType::Signal(payload)
                                            if payload.same_shape(expected_parameter)
                                    ))
                        })
                        || expression_matches(self.module, *argument, env, expected_parameter)
                });
            if !explicit_arguments_match {
                continue;
            }
            matches.push(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments: explicit_arguments.clone(),
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }
        if matches.len() != 1 {
            return None;
        }
        matches.pop()
    }

    /// Match a pipe stage body against import-backed functions (single `Import` or
    /// `AmbiguousHoistedImports`). Walks each candidate's curried Arrow type through
    /// the explicit argument types and then the ambient subject type. Returns the unique
    /// `PipeFunctionSignatureMatch` when exactly one candidate matches.
    fn match_import_pipe_function_signature(
        &mut self,
        callee_expr: ExprId,
        import_candidates: Vec<ImportId>,
        explicit_arguments: Vec<ExprId>,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let explicit_arg_types: Vec<Option<GateType>> = explicit_arguments
            .iter()
            .map(|arg| {
                let info = self.infer_expr(*arg, env, Some(ambient));
                info.actual_gate_type().or(info.ty)
            })
            .collect();

        let mut result_matches: Vec<(Vec<GateType>, GateType)> = Vec::new();

        for import_id in import_candidates {
            let Some(import_ty) = self.import_value_type_with_ambient(import_id) else {
                continue;
            };
            if !matches!(import_ty, GateType::Arrow { .. }) {
                continue;
            }

            let mut current = import_ty;
            let mut param_types: Vec<GateType> = Vec::new();
            let mut ok = true;

            for arg_ty_opt in &explicit_arg_types {
                match arg_ty_opt {
                    Some(arg_ty) => {
                        let Some(next) = self.apply_function(&current, arg_ty) else {
                            ok = false;
                            break;
                        };
                        // Record the concrete parameter type (substituting any TypeParams
                        // with the concrete argument type).
                        let concrete_param = match &current {
                            GateType::Arrow { parameter, .. } => {
                                if parameter.has_type_params() {
                                    arg_ty.clone()
                                } else {
                                    *parameter.clone()
                                }
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        param_types.push(concrete_param);
                        current = next;
                    }
                    None => {
                        // Unknown arg type — advance past this Arrow position.
                        let GateType::Arrow { parameter, result } = current else {
                            ok = false;
                            break;
                        };
                        param_types.push(*parameter);
                        current = *result;
                    }
                }
            }

            if !ok {
                continue;
            }

            let Some(result_ty) = self.apply_function(&current, ambient) else {
                continue;
            };

            if let Some(expected) = expected_result
                && !result_ty.same_shape(expected) {
                    continue;
                }

            let concrete_ambient_param = match current {
                GateType::Arrow { parameter, .. } => {
                    if parameter.has_type_params() {
                        ambient.clone()
                    } else {
                        *parameter
                    }
                }
                _ => continue,
            };
            param_types.push(concrete_ambient_param);
            result_matches.push((param_types, result_ty));
        }

        if result_matches.len() != 1 {
            return None;
        }

        let (parameter_types, result_type) = result_matches.pop().unwrap();
        let n_explicit = explicit_arguments.len();
        Some(PipeFunctionSignatureMatch {
            callee_expr,
            explicit_arguments,
            signal_payload_arguments: vec![false; n_explicit],
            parameter_types,
            result_type,
        })
    }

    pub(crate) fn match_pipe_unannotated_function_signature(
        &mut self,
        callee_expr: ExprId,
        explicit_arguments: &[ExprId],
        function: &crate::hir::FunctionItem,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let mut bindings = PolyTypeBindings::new();
        let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
        let mut explicit_argument_types = Vec::with_capacity(explicit_arguments.len());

        for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_ty = argument_info.actual_gate_type().or(argument_info.ty);
            if let Some(annotation) = parameter.annotation {
                if let Some(argument_ty) = argument_ty.as_ref() {
                    let reads_signal_payload = self.match_pipe_argument_parameter_annotation(
                        annotation,
                        argument_ty,
                        &mut bindings,
                    )?;
                    signal_payload_arguments.push(reads_signal_payload);
                } else {
                    signal_payload_arguments.push(false);
                }
            } else {
                signal_payload_arguments.push(false);
            }
            explicit_argument_types.push(argument_ty);
        }

        let ambient_parameter = function
            .parameters
            .last()
            .expect("checked pipe arity above");
        if let Some(annotation) = ambient_parameter.annotation {
            self.match_function_parameter_annotation(annotation, ambient, &mut bindings)?;
        }

        if let Some(result_annotation) = function.annotation
            && let Some(expected) = expected_result
        {
            self.match_function_parameter_annotation(result_annotation, expected, &mut bindings)?;
        }

        let mut parameter_types = Vec::with_capacity(function.parameters.len());
        for (index, parameter) in function.parameters.iter().enumerate() {
            let parameter_ty = if let Some(annotation) = parameter.annotation {
                self.instantiate_function_parameter_annotation(annotation, &bindings)?
            } else if index < explicit_argument_types.len() {
                explicit_argument_types[index].clone()?
            } else {
                ambient.clone()
            };
            parameter_types.push(parameter_ty);
        }

        let mut function_env = GateExprEnv::default();
        for (parameter, parameter_ty) in function.parameters.iter().zip(parameter_types.iter()) {
            function_env
                .locals
                .insert(parameter.binding, parameter_ty.clone());
        }

        let result_type = if let Some(result_annotation) = function.annotation {
            self.instantiate_function_parameter_annotation(result_annotation, &bindings)?
        } else {
            let body_info = self.infer_expr(function.body, &function_env, None);
            body_info.actual_gate_type().or(body_info.ty)?
        };
        if let Some(expected) = expected_result
            && !result_type.same_shape(expected)
        {
            return None;
        }

        if let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind
            && let ResolutionState::Resolved(TermResolution::Item(item_id)) =
                reference.resolution.as_ref()
            && let Item::Function(function_item) = &self.module.items()[*item_id]
            && supports_same_module_function_inference(function_item)
        {
            self.record_function_signature_evidence(FunctionSignatureEvidence {
                item_id: *item_id,
                parameter_types: parameter_types.clone(),
                result_type: result_type.clone(),
            });
        }

        Some(PipeFunctionSignatureMatch {
            callee_expr,
            explicit_arguments: explicit_arguments.to_vec(),
            signal_payload_arguments,
            parameter_types,
            result_type,
        })
    }

    pub(crate) fn infer_polymorphic_function_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return None;
        };
        if function.type_parameters.is_empty() {
            return None;
        }
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let Some((_, result)) = self.match_function_signature(function, &argument_types, None) {
            info.ty = Some(result);
        }
        Some(info)
    }

    /// Infer the result type of applying an imported function to explicit arguments.
    ///
    /// When calling an imported function like `withSelectedThreadId None`, the generic
    /// `infer_expr` path cannot determine `None`'s type without the Arrow parameter context.
    /// This function uses the import's Arrow chain to provide expected types for each argument,
    /// falling back to the Arrow parameter type when argument inference returns `None` (which is
    /// safe because the HIR type-checker has already validated the call).
    pub(crate) fn infer_import_function_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        _ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Import(import_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let import_ty = self.import_value_type(*import_id)?;
        // Only handle Arrow types (function applications)
        if !matches!(import_ty, GateType::Arrow { .. }) {
            return None;
        }
        let mut info = GateExprInfo::default();
        let mut current = import_ty;
        for argument in arguments.iter() {
            // Extract parameter type from current Arrow for use as context
            let (param, fallback_result) = match &current {
                GateType::Arrow { parameter, result } => {
                    (parameter.as_ref().clone(), result.as_ref().clone())
                }
                _ => return Some(info),
            };
            // Infer argument using the Arrow parameter as context
            let arg_info = self.infer_expr(*argument, env, Some(&param));
            // Use inferred type, falling back to the parameter type when unknown.
            // This is safe because HIR type-checking has already validated the call.
            let arg_ty = arg_info
                .actual_gate_type()
                .or(arg_info.ty.clone())
                .unwrap_or_else(|| param.clone());
            info.merge(arg_info);
            // Advance the Arrow chain by applying this argument
            current = self
                .apply_function(&current, &arg_ty)
                .unwrap_or(fallback_result);
        }
        if info.issues.is_empty() {
            info.ty = Some(current);
        }
        Some(info)
    }

    pub(crate) fn infer_class_member_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.class_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let DomainMemberSelection::Unique(matched) =
            self.select_class_member_call(reference, &argument_types, None)?
        {
            info.ty = Some(matched.result);
        }
        Some(info)
    }

    pub(crate) fn infer_domain_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.domain_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        match self.select_domain_member_call(reference, &argument_types, None)? {
            DomainMemberSelection::Unique(matched) => {
                info.ty = Some(matched.result);
            }
            DomainMemberSelection::Ambiguous => {
                info.issues.push(GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                });
            }
            DomainMemberSelection::NoMatch => {}
        }
        Some(info)
    }

    pub(crate) fn infer_pipe_body_inference(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> PipeBodyInference {
        let ambient = subject.gate_payload().clone();
        let mut info = self.infer_expr(expr_id, env, Some(&ambient));
        let mut transform_mode = PipeTransformMode::Replace;
        if let Some(function_body) = self.infer_function_pipe_body(expr_id, env, &ambient, None) {
            info = function_body;
            transform_mode = PipeTransformMode::Apply;
        } else if let Some(GateType::Arrow { parameter, result }) = info.ty.clone() {
            if parameter.same_shape(&ambient) {
                info.ty = Some(*result);
                transform_mode = PipeTransformMode::Apply;
            } else {
                info.issues.push(GateIssue::InvalidPipeStageInput {
                    span: self.module.exprs()[expr_id].span,
                    stage: "pipe",
                    expected: ambient.to_string(),
                    actual: parameter.to_string(),
                });
                info.ty = None;
            }
        } else if info.ty.is_none() && info.issues.is_empty() {
            // Fallback: try to infer the result type for import-backed pipe stages
            // (e.g. `|> map f` where `map` is an ambiguous hoisted import).  The
            // earlier paths only handle same-module functions and class members; this
            // path covers the import / hoisted-import case by walking the Arrow chain
            // of each candidate import and returning the unique matching result.
            if let Some(result_ty) = self.infer_import_apply_pipe_result(expr_id, env, &ambient) {
                info.ty = Some(result_ty);
                transform_mode = PipeTransformMode::Apply;
            }
        }
        PipeBodyInference {
            info,
            transform_mode,
        }
    }

    /// Try to infer the result type of a pipe stage that is an imported function
    /// applied to explicit arguments, with the pipe subject as the last argument.
    /// Handles both a single resolved import and `AmbiguousHoistedImports`,
    /// returning the unique result when exactly one candidate successfully type-checks.
    fn infer_import_apply_pipe_result(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
    ) -> Option<GateType> {
        let (callee_expr, explicit_arguments) = self.flatten_apply_expr(expr_id);

        // Collect candidate import IDs from the callee resolution (owned Vec to free the borrow).
        let candidates: Vec<ImportId> = {
            let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind else {
                return None;
            };
            match reference.resolution.as_ref() {
                ResolutionState::Resolved(TermResolution::Import(id)) => vec![*id],
                ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(ids)) => {
                    ids.iter().copied().collect()
                }
                _ => return None,
            }
        };

        // Infer types for explicit arguments (needs &mut self, safe because candidates is owned).
        let explicit_arg_types: Vec<Option<GateType>> = explicit_arguments
            .iter()
            .map(|arg| {
                let arg_info = self.infer_expr(*arg, env, None);
                arg_info.actual_gate_type().or(arg_info.ty)
            })
            .collect();

        // For each candidate: walk the Arrow chain through explicit args, then apply ambient.
        let mut results = Vec::new();
        for import_id in candidates {
            let import_ty = match self.import_value_type_with_ambient(import_id) {
                Some(ty) if matches!(ty, GateType::Arrow { .. }) => ty,
                _ => continue,
            };
            let mut current = import_ty;
            let mut ok = true;
            for arg_ty_opt in &explicit_arg_types {
                match arg_ty_opt {
                    Some(arg_ty) => match self.apply_function(&current, arg_ty) {
                        Some(next) => current = next,
                        None => {
                            ok = false;
                            break;
                        }
                    },
                    None => {
                        // Unknown arg type — advance past the Arrow parameter.
                        match current {
                            GateType::Arrow { result, .. } => current = *result,
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            if let Some(result_ty) = self.apply_function(&current, ambient) {
                results.push(result_ty);
            }
        }

        if results.len() == 1 {
            results.pop()
        } else {
            None
        }
    }

    pub(crate) fn infer_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let inference = self.infer_pipe_body_inference(expr_id, env, subject);
        self.finalize_expr_info(inference.info)
    }

    pub(crate) fn infer_tap_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        info.ty = Some(subject.clone());
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_accumulate_stage_info(
        &mut self,
        seed_expr: ExprId,
        step_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let seed_info = self.infer_expr(seed_expr, env, None);
        let seed_ty = seed_info.ty.clone();
        info.merge(seed_info);
        let Some(seed_ty) = seed_ty else {
            return self.finalize_expr_info(info);
        };

        let step_info = self.infer_expr(step_expr, env, Some(input_payload.as_ref()));
        let step_ty = step_info.actual_gate_type().or(step_info.ty.clone());
        info.merge(step_info);
        let Some(step_ty) = step_ty else {
            return self.finalize_expr_info(info);
        };

        let Some((parameters, result_ty)) = self.function_signature(&step_ty, 2) else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: format!("{} -> {} -> {}", input_payload.as_ref(), seed_ty, seed_ty),
                actual: step_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        };
        if !parameters[0].same_shape(input_payload.as_ref())
            || !parameters[1].same_shape(&seed_ty)
            || !result_ty.same_shape(&seed_ty)
        {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: format!("{} -> {} -> {}", input_payload.as_ref(), seed_ty, seed_ty),
                actual: step_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(Box::new(seed_ty)));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_previous_stage_info(
        &mut self,
        seed_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[seed_expr].span,
                stage: "~|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let seed_info = self.infer_expr(seed_expr, env, None);
        let seed_ty = seed_info.actual_gate_type().or(seed_info.ty.clone());
        info.merge(seed_info);
        let Some(seed_ty) = seed_ty else {
            return self.finalize_expr_info(info);
        };
        if !seed_ty.same_shape(input_payload.as_ref()) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[seed_expr].span,
                stage: "~|>",
                expected: input_payload.as_ref().to_string(),
                actual: seed_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(Box::new(seed_ty)));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_diff_stage_info(
        &mut self,
        diff_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[diff_expr].span,
                stage: "-|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let stage_info = self.infer_expr(diff_expr, env, None);
        let stage_ty = stage_info.actual_gate_type().or(stage_info.ty.clone());
        info.merge(stage_info);
        let Some(stage_ty) = stage_ty else {
            return self.finalize_expr_info(info);
        };

        if let Some((parameters, result_ty)) = self.function_signature(&stage_ty, 2) {
            if !parameters[0].same_shape(input_payload.as_ref())
                || !parameters[1].same_shape(input_payload.as_ref())
                || result_ty.is_signal()
            {
                info.issues.push(GateIssue::InvalidPipeStageInput {
                    span: self.module.exprs()[diff_expr].span,
                    stage: "-|>",
                    expected: format!(
                        "{} -> {} -> _",
                        input_payload.as_ref(),
                        input_payload.as_ref()
                    ),
                    actual: stage_ty.to_string(),
                });
                return self.finalize_expr_info(info);
            }
            info.ty = Some(GateType::Signal(Box::new(result_ty.clone())));
            return self.finalize_expr_info(info);
        }

        if stage_ty.same_shape(input_payload.as_ref()) && is_numeric_gate_type(input_payload) {
            info.ty = Some(GateType::Signal(Box::new(stage_ty)));
            return self.finalize_expr_info(info);
        }

        info.issues.push(GateIssue::InvalidPipeStageInput {
            span: self.module.exprs()[diff_expr].span,
            stage: "-|>",
            expected: format!(
                "{} -> {} -> _  or seeded {}",
                input_payload.as_ref(),
                input_payload.as_ref(),
                input_payload.as_ref()
            ),
            actual: stage_ty.to_string(),
        });
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_delay_stage_info(
        &mut self,
        duration_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[duration_expr].span,
                stage: "|> delay",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let duration_info = self.infer_expr(duration_expr, env, None);
        let duration_ty = duration_info
            .actual_gate_type()
            .or(duration_info.ty.clone());
        info.merge(duration_info);
        let Some(duration_ty) = duration_ty else {
            return self.finalize_expr_info(info);
        };
        if !is_duration_gate_type(&duration_ty) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[duration_expr].span,
                stage: "|> delay",
                expected: "Duration".to_owned(),
                actual: duration_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(input_payload.clone()));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_burst_stage_info(
        &mut self,
        every_expr: ExprId,
        count_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[every_expr].span,
                stage: "|> burst",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let every_info = self.infer_expr(every_expr, env, None);
        let every_ty = every_info.actual_gate_type().or(every_info.ty.clone());
        info.merge(every_info);
        let Some(every_ty) = every_ty else {
            return self.finalize_expr_info(info);
        };
        if !is_duration_gate_type(&every_ty) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[every_expr].span,
                stage: "|> burst",
                expected: "Duration".to_owned(),
                actual: every_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        let count_info = self.infer_expr(count_expr, env, None);
        let count_ty = count_info.actual_gate_type().or(count_info.ty.clone());
        info.merge(count_info);
        let Some(count_ty) = count_ty else {
            return self.finalize_expr_info(info);
        };
        if !is_burst_count_gate_type(&count_ty) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[count_expr].span,
                stage: "|> burst",
                expected: "Int or Retry (for example `3times`)".to_owned(),
                actual: count_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(input_payload.clone()));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_truthy_falsy_branch(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        payload_subject: Option<&GateType>,
        subject: &GateType,
    ) -> GateExprInfo {
        match payload_subject {
            Some(subject) => self.infer_pipe_body(expr_id, env, subject),
            None if subject.is_signal() => {
                self.infer_expr(expr_id, env, Some(subject.gate_payload()))
            }
            None => self.infer_expr(expr_id, env, None),
        }
    }

    pub(crate) fn infer_truthy_falsy_pair(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let subject_plan = self.truthy_falsy_subject_plan(subject)?;
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
            subject,
        );
        if !truthy.issues.is_empty() {
            return None;
        }
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
            subject,
        );
        if !falsy.issues.is_empty() {
            return None;
        }
        let truthy_ty = truthy.actual()?;
        let falsy_ty = falsy.actual()?;
        self.apply_truthy_falsy_result_actual(subject, truthy_ty.unify(&falsy_ty)?)
            .to_gate_type()
    }

    pub(crate) fn infer_truthy_falsy_pair_info(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(subject_plan) = self.truthy_falsy_subject_plan(subject) else {
            return GateExprInfo::default();
        };
        let mut info = GateExprInfo::default();
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
            subject,
        );
        let truthy_ty = truthy.actual();
        info.merge(truthy);
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
            subject,
        );
        let falsy_ty = falsy.actual();
        info.merge(falsy);
        if info.issues.is_empty()
            && let (Some(truthy_ty), Some(falsy_ty)) = (truthy_ty, falsy_ty)
                && let Some(branch_ty) = truthy_ty.unify(&falsy_ty) {
                    info.set_actual(self.apply_truthy_falsy_result_actual(subject, branch_ty));
                }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_function_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let plan = self.match_pipe_function_signature(expr_id, env, ambient, expected_result)?;
        let mut info = GateExprInfo::default();
        for ((argument, expected), reads_signal_payload) in plan
            .explicit_arguments
            .iter()
            .zip(
                plan.parameter_types
                    .iter()
                    .take(plan.explicit_arguments.len()),
            )
            .zip(plan.signal_payload_arguments.iter())
        {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_actual = argument_info.actual_gate_type();
            let argument_annot = argument_info.ty.clone();
            // Prefer actual type; also keep the annotated type as a fallback for
            // generic functions whose actual inference produces Hole placeholders
            // in place of type parameters (e.g. `lengthStep : Int -> A -> Int`).
            let argument_ty = argument_actual.or(argument_annot.clone());
            info.merge(argument_info);
            let matches_expected = argument_ty.as_ref().is_some_and(|actual| {
                actual.same_shape(expected)
                    || (*reads_signal_payload
                        && matches!(
                            actual,
                            GateType::Signal(payload) if payload.same_shape(expected)
                        ))
            }) || argument_annot
                .as_ref()
                .is_some_and(|ty| ty.same_shape(expected))
                || expression_matches(self.module, *argument, env, expected);
            if !matches_expected {
                return Some(info);
            }
        }
        if info.issues.is_empty() {
            info.ty = Some(plan.result_type);
        }
        Some(info)
    }

    pub(crate) fn infer_gate_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_gate_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_fanout_map_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanout_map_stage_info(expr_id, env, subject).ty
    }

    /// Infer only the result type for a full fanout segment, without building
    /// filter plans or join plans. Used by grouped subject walkers to advance
    /// the subject type past `*|>` map/filter(/join) runs without re-running
    /// the full `elaborate_fanout_segment` pass that validation already
    /// performed (PA-H2).
    pub(crate) fn infer_fanout_segment_result_type(
        &mut self,
        segment: &crate::PipeFanoutSegment<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let carrier = self.fanout_carrier(subject)?;
        let element_subject = subject.fanout_element().cloned()?;
        let map_env = pipe_stage_expr_env(env, segment.map_stage(), subject);
        let mapped_element_type = self
            .infer_pipe_body(segment.map_expr(), &map_env, &element_subject)
            .ty?;
        let mapped_collection_type = self.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Map, carrier),
            mapped_element_type,
        );
        let mut segment_env = env.clone();
        extend_pipe_env_with_stage_result_memo(
            &mut segment_env,
            segment.map_stage(),
            &mapped_collection_type,
        );
        for stage in segment.filter_stages() {
            extend_pipe_env_with_stage_result_memo(&mut segment_env, stage, &mapped_collection_type);
        }
        if let Some(join_expr) = segment.join_expr() {
            let join_env = pipe_stage_expr_env(
                &segment_env,
                segment
                    .join_stage()
                    .expect("join expression implies join stage"),
                &mapped_collection_type,
            );
            let join_value_type = self
                .infer_pipe_body(join_expr, &join_env, &mapped_collection_type)
                .ty?;
            Some(self.apply_fanout_plan(
                FanoutPlanner::plan(FanoutStageKind::Join, carrier),
                join_value_type,
            ))
        } else {
            Some(mapped_collection_type)
        }
    }

    pub(crate) fn infer_fanin_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanin_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_transform_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_transform_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_transform_stage_mode(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> PipeTransformMode {
        self.infer_pipe_body_inference(expr_id, env, subject)
            .transform_mode
    }

    pub(crate) fn infer_transform_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let PipeBodyInference {
            mut info,
            transform_mode: _,
        } = self.infer_pipe_body_inference(expr_id, env, subject);
        let body_ty = info.actual_gate_type().or(info.ty.clone());
        info.ty = body_ty.map(|body_ty| match subject {
            GateType::Signal(_) => GateType::Signal(Box::new(body_ty)),
            _ => body_ty,
        });
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_validate_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let plan = subject.validate_stage_subject();
        let mut info = self.infer_pipe_body(expr_id, env, plan.input_subject());
        let Some(body_ty) = info.actual_gate_type().or(info.ty.clone()) else {
            return self.finalize_expr_info(info);
        };
        if !plan.accepts_result(&body_ty) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[expr_id].span,
                stage: "!|>",
                expected: plan.expected_result_description(),
                actual: body_ty.to_string(),
            });
            info.ty = None;
            return self.finalize_expr_info(info);
        }
        info.ty = Some(match subject {
            GateType::Signal(_) => GateType::Signal(Box::new(body_ty)),
            _ => body_ty,
        });
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_gate_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        let is_valid = info.issues.is_empty()
            && !info.contains_signal
            && !info.ty.as_ref().is_some_and(GateType::is_signal)
            && info.ty.as_ref().is_some_and(GateType::is_bool);
        info.ty = is_valid
            .then(|| self.apply_gate_plan(GatePlanner::plan(subject.gate_carrier()), subject));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_fanout_map_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let Some(element) = subject.fanout_element() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, element);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Map, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_fanin_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Join, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_case_stage_run_info(
        &mut self,
        case_run: &crate::PipeCaseStageRun<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let mut branch_result = None::<SourceOptionActualType>;
        let branch_subject = subject.gate_payload().clone();
        let case_env = pipe_stage_expr_env(env, case_run.start_stage(), subject);
        for stage in case_run.stages() {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                continue;
            };
            let mut branch_env = case_env.clone();
            branch_env
                .locals
                .extend(self.case_pattern_bindings(*pattern, &branch_subject).locals);
            let branch = self.infer_pipe_body(*body, &branch_env, &branch_subject);
            let branch_ty = branch.actual();
            info.merge(branch);
            let Some(branch_ty) = branch_ty else {
                continue;
            };
            match branch_result.as_ref() {
                None => branch_result = Some(branch_ty),
                Some(current) => {
                    let Some(unified) = current.unify(&branch_ty) else {
                        info.issues.push(GateIssue::CaseBranchTypeMismatch {
                            span: stage.span,
                            expected: current.to_string(),
                            actual: branch_ty.to_string(),
                        });
                        branch_result = None;
                        break;
                    };
                    branch_result = Some(unified);
                }
            }
        }
        if info.issues.is_empty()
            && let Some(branch_result) = branch_result {
                info.set_actual(match subject.gate_carrier() {
                    GateCarrier::Ordinary => branch_result,
                    GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(branch_result)),
                });
            }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_cluster_expr(
        &mut self,
        cluster_id: ClusterId,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        let Some(cluster) = self.module.clusters().get(cluster_id).cloned() else {
            return GateExprInfo::default();
        };
        let spine = cluster.normalized_spine();
        let mut info = GateExprInfo::default();
        let mut cluster_kind = None::<ApplicativeClusterKind>;
        let mut payloads = Vec::new();

        for member in spine.apply_arguments() {
            let member_info = self.infer_expr(member, env, None);
            let member_ty = member_info.actual();
            info.merge(member_info);
            let Some(member_ty) = member_ty else {
                return self.finalize_expr_info(info);
            };
            let Some((member_kind, payload)) =
                ApplicativeClusterKind::from_member_actual(&member_ty)
            else {
                info.issues
                    .push(GateIssue::UnsupportedApplicativeClusterMember {
                        span: self.module.exprs()[member].span,
                        actual: member_ty.to_string(),
                    });
                return self.finalize_expr_info(info);
            };
            match cluster_kind.as_ref() {
                None => {
                    cluster_kind = Some(member_kind);
                    payloads.push(payload);
                }
                Some(expected) => {
                    let Some(unified) = expected.unify(&member_kind) else {
                        info.issues.push(GateIssue::ApplicativeClusterMismatch {
                            span: self.module.exprs()[member].span,
                            expected: expected.surface(),
                            actual: member_kind.surface(),
                        });
                        return self.finalize_expr_info(info);
                    };
                    cluster_kind = Some(unified);
                    payloads.push(payload);
                }
            }
        }

        let Some(cluster_kind) = cluster_kind else {
            return self.finalize_expr_info(info);
        };
        let payload_result = match spine.pure_head() {
            ApplicativeSpineHead::TupleConstructor(_) => SourceOptionActualType::Tuple(payloads),
            ApplicativeSpineHead::Expr(finalizer) => {
                let finalizer_info = self.infer_expr(finalizer, env, None);
                let finalizer_ty = finalizer_info.ty.clone();
                let finalizer_had_issues = !finalizer_info.issues.is_empty();
                info.merge(finalizer_info);

                let closed_payloads = payloads
                    .iter()
                    .map(SourceOptionActualType::to_gate_type)
                    .collect::<Option<Vec<_>>>();
                let applied_from_type = finalizer_ty
                    .as_ref()
                    .zip(closed_payloads.as_ref())
                    .and_then(|(ty, payloads)| self.apply_function_chain(ty, payloads));
                let applied_from_constructor = match &self.module.exprs()[finalizer].kind {
                    ExprKind::Name(reference) => {
                        let from_builtin = self
                            .infer_builtin_constructor_actual_from_reference(reference, &payloads);
                        let from_same_module =
                            self.infer_same_module_constructor_apply_actual(reference, &payloads);
                        from_builtin.or(from_same_module).or_else(|| {
                            closed_payloads.as_ref().and_then(|payloads| {
                                self.infer_same_module_constructor_apply(reference, payloads)
                                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                            })
                        })
                    }
                    _ => None,
                };

                match applied_from_type
                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                    .or(applied_from_constructor)
                {
                    Some(result) => result,
                    None => {
                        if !finalizer_had_issues {
                            info.issues.push(GateIssue::InvalidClusterFinalizer {
                                span: self.module.exprs()[finalizer].span,
                                expected_inputs: payloads.iter().map(ToString::to_string).collect(),
                                actual: finalizer_ty
                                    .map(|ty| ty.to_string())
                                    .unwrap_or_else(|| "unknown type".to_owned()),
                            });
                        }
                        return self.finalize_expr_info(info);
                    }
                }
            }
        };
        info.set_actual(cluster_kind.wrap_actual(payload_result));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_pipe_expr(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        let mut info = self.infer_expr(pipe.head, env, None);
        let mut current = info.ty.clone();
        let mut pipe_env = env.clone();
        for semantic_stage in pipe.semantic_stages() {
            let stage = semantic_stage.start_stage();
            let Some(subject) = current.clone() else {
                break;
            };
            let stage_info = match semantic_stage {
                crate::PipeSemanticStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Transform { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_transform_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Tap { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_tap_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Gate { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_gate_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Map { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_fanout_map_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::FanIn { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_fanin_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Accumulate { seed, step } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_accumulate_stage_info(*seed, *step, &stage_env, &subject)
                    }
                    PipeStageKind::Previous { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_previous_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Diff { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_diff_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Delay { duration } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_delay_stage_info(*duration, &stage_env, &subject)
                    }
                    PipeStageKind::Burst { every, count } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_burst_stage_info(*every, *count, &stage_env, &subject)
                    }
                    PipeStageKind::Apply { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. } => GateExprInfo::default(),
                    PipeStageKind::Validate { expr } => {
                        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                        self.infer_validate_stage_info(*expr, &stage_env, &subject)
                    }
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Case { .. } => {
                        unreachable!("semantic stage iterator groups truthy/falsy pairs and case runs")
                    }
                },
                crate::PipeSemanticStage::TruthyFalsyPair(pair) => {
                    let pair_env = pipe_stage_expr_env(&pipe_env, pair.start_stage(), &subject);
                    self.infer_truthy_falsy_pair_info(&pair, &pair_env, &subject)
                }
                crate::PipeSemanticStage::CaseRun(case_run) => {
                    self.infer_case_stage_run_info(&case_run, &pipe_env, &subject)
                }
            };
            let result_subject = stage_info.actual_gate_type().or(stage_info.ty.clone());
            if let Some(result_subject) = result_subject.as_ref() {
                extend_pipe_env_with_stage_memos(&mut pipe_env, stage, &subject, result_subject);
            }
            current = result_subject;
            info.merge(stage_info);
        }
        info.ty = current;
        info
    }

    pub(crate) fn project_type(
        &mut self,
        subject: &GateType,
        path: &NamePath,
        current_domain: Option<ItemId>,
    ) -> Result<GateType, GateIssue> {
        let mut current = subject.clone();
        for segment in path.segments().iter() {
            current = self
                .project_type_step(&current, segment, path, current_domain)?
                .result()
                .clone();
        }
        Ok(current)
    }

    pub(crate) fn project_type_step(
        &mut self,
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
        current_domain: Option<ItemId>,
    ) -> Result<GateProjectionStep, GateIssue> {
        match subject {
            GateType::Record(fields) => {
                self.project_record_field_step(fields, false, subject, segment, path)
            }
            GateType::Signal(payload) => match payload.as_ref() {
                GateType::Record(fields) => {
                    self.project_record_field_step(fields, true, subject, segment, path)
                }
                _ => Err(GateIssue::InvalidProjection {
                    span: path.span(),
                    path: name_path_text(path),
                    subject: subject.to_string(),
                }),
            },
            GateType::Domain {
                item, arguments, ..
            } => self.project_domain_member_step(
                *item,
                arguments,
                subject,
                segment,
                path,
                current_domain,
            ),
            GateType::OpaqueImport { import, .. } => {
                // Guard against the sentinel value (u32::MAX) used when a named type was not
                // found in the current module's imports — `get` returns None safely.
                let Some(import_binding) = self.module.imports().get(*import) else {
                    return Err(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(path),
                        subject: subject.to_string(),
                    });
                };
                // Dispatch on the import metadata.
                match &import_binding.metadata {
                    ImportBindingMetadata::TypeConstructor {
                        fields: Some(fields),
                        ..
                    } => {
                        // Imported record type: resolve the named field.
                        let import_fields = fields
                            .iter()
                            .map(|f| GateRecordField {
                                name: f.name.to_string(),
                                ty: self.lower_import_value_type(&f.ty),
                            })
                            .collect::<Vec<_>>();
                        self.project_record_field_step(
                            &import_fields,
                            false,
                            subject,
                            segment,
                            path,
                        )
                    }
                    _ => Err(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(path),
                        subject: subject.to_string(),
                    }),
                }
            }
            _ => Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
        }
    }

    pub(crate) fn project_record_field_step(
        &self,
        fields: &[GateRecordField],
        wrap_signal: bool,
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
    ) -> Result<GateProjectionStep, GateIssue> {
        let Some(field) = fields.iter().find(|field| field.name == segment.text()) else {
            return Err(GateIssue::UnknownField {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            });
        };
        let result = if wrap_signal {
            GateType::Signal(Box::new(field.ty.clone()))
        } else {
            field.ty.clone()
        };
        Ok(GateProjectionStep::RecordField { result })
    }

    pub(crate) fn project_domain_member_step(
        &mut self,
        domain_item: ItemId,
        domain_arguments: &[GateType],
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
        current_domain: Option<ItemId>,
    ) -> Result<GateProjectionStep, GateIssue> {
        let Item::Domain(domain) = &self.module.items()[domain_item] else {
            return Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            });
        };

        if segment.text() == "carrier" && current_domain == Some(domain_item) {
            let carrier_ty = self.lower_open_annotation(domain.carrier);
            if let Some(mut carrier_ty) = carrier_ty {
                let substitutions: HashMap<_, _> = domain
                    .parameters
                    .iter()
                    .copied()
                    .zip(domain_arguments.iter().cloned())
                    .collect();
                if !substitutions.is_empty() {
                    carrier_ty = carrier_ty.substitute_type_parameters(&substitutions);
                }
                let handle = DomainMemberHandle {
                    domain: domain_item,
                    domain_name: domain.name.text().into(),
                    member_name: "carrier".into(),
                    member_index: crate::hir::SYNTHETIC_DOMAIN_CARRIER_MEMBER_INDEX,
                };
                return Ok(GateProjectionStep::DomainMember {
                    handle,
                    result: carrier_ty,
                });
            }
        }

        let substitutions = domain
            .parameters
            .iter()
            .copied()
            .zip(domain_arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut matches = Vec::new();
        let mut found_named_member = false;
        for (member_index, member) in domain.members.iter().enumerate() {
            if member.kind != DomainMemberKind::Method || member.name.text() != segment.text() {
                continue;
            }
            found_named_member = true;
            let resolution = DomainMemberResolution {
                domain: domain_item,
                member_index,
            };
            let Some(annotation) = self.lower_domain_member_annotation(resolution, &substitutions)
            else {
                continue;
            };
            let Some((parameters, result)) = self.function_signature(&annotation, 1) else {
                continue;
            };
            let Some(parameter) = parameters.first() else {
                continue;
            };
            if !parameter.same_shape(subject) {
                continue;
            }
            let Some(handle) = self.module.domain_member_handle(resolution) else {
                continue;
            };
            matches.push((handle, result));
        }

        match matches.len() {
            1 => {
                let (handle, result) = matches
                    .pop()
                    .expect("exactly one domain projection match should be available");
                Ok(GateProjectionStep::DomainMember { handle, result })
            }
            0 if found_named_member => Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
            0 => Err(GateIssue::UnknownField {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
            _ => Err(GateIssue::AmbiguousDomainMember {
                span: path.span(),
                name: segment.text().to_owned(),
                candidates: matches
                    .into_iter()
                    .map(|(handle, _)| format!("{}.{}", handle.domain_name, handle.member_name))
                    .collect(),
            }),
        }
    }

    pub(crate) fn apply_function(
        &self,
        callee: &GateType,
        argument: &GateType,
    ) -> Option<GateType> {
        let GateType::Arrow { parameter, result } = callee else {
            return None;
        };
        if parameter.same_shape(argument) {
            return Some(result.as_ref().clone());
        }
        // Polymorphic application: if the parameter is an open type variable, substitute it in
        // the result to produce a concrete return type without requiring exact structural equality.
        if let GateType::TypeParameter {
            parameter: param_id,
            ..
        } = parameter.as_ref()
        {
            return Some(result.substitute_type_parameter(*param_id, argument));
        }
        // Structural unification: the parameter may be a compound type containing TypeParameter
        // nodes (e.g. OpaqueImport(NEL, [TypeParam(A)])). Match structurally against the argument,
        // collect bindings, and substitute them into the result.
        if parameter.has_type_params() {
            let mut bindings = HashMap::new();
            if argument.unify_type_params(parameter, &mut bindings) && !bindings.is_empty() {
                return Some(result.substitute_type_parameters(&bindings));
            }
        }
        None
    }

    pub(crate) fn apply_function_chain(
        &self,
        callee: &GateType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        let mut current = callee.clone();
        for argument in arguments {
            current = self.apply_function(&current, argument)?;
        }
        Some(current)
    }
}

pub(crate) fn is_numeric_gate_type(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(
            BuiltinType::Int | BuiltinType::Float | BuiltinType::Decimal | BuiltinType::BigInt
        )
    )
}

pub(crate) fn is_duration_gate_type(ty: &GateType) -> bool {
    ty.has_named_type("Duration")
}

pub(crate) fn is_burst_count_gate_type(ty: &GateType) -> bool {
    matches!(ty, GateType::Primitive(BuiltinType::Int))
        || ty.has_named_type("Retry")
}

pub(crate) fn name_path_text(path: &NamePath) -> String {
    format!(
        ".{}",
        path.segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".")
    )
}

pub(crate) fn case_constructor_key(reference: &TermReference) -> Option<CaseConstructorKey> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
            Some(CaseConstructorKey::Builtin(*builtin))
        }
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            Some(CaseConstructorKey::SameModuleVariant {
                item: *item_id,
                name: reference.path.segments().iter().last()?.text().to_owned(),
            })
        }
        ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            Some(CaseConstructorKey::ImportedVariant {
                import: *import_id,
                name: reference.path.segments().iter().last()?.text().to_owned(),
            })
        }
        ResolutionState::Unresolved
        | ResolutionState::Resolved(TermResolution::Local(_))
        | ResolutionState::Resolved(TermResolution::DomainConstructor(_))
        | ResolutionState::Resolved(TermResolution::DomainMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
        | ResolutionState::Resolved(TermResolution::ClassMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
        | ResolutionState::Resolved(TermResolution::IntrinsicValue(_)) => None,
    }
}

pub(crate) fn missing_case_list(missing: &[CaseConstructorShape]) -> String {
    missing
        .iter()
        .map(|constructor| format!("`{}`", constructor.display))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn missing_case_label(missing: &[CaseConstructorShape]) -> String {
    let cases = missing_case_list(missing);
    if missing.len() == 1 {
        format!("add a case for {cases}, or use `_` to make the catch-all explicit")
    } else {
        format!("add cases for {cases}, or use `_` to make the catch-all explicit")
    }
}

pub(crate) fn custom_source_wakeup_kind(
    wakeup: CustomSourceRecurrenceWakeup,
) -> RecurrenceWakeupKind {
    match wakeup {
        CustomSourceRecurrenceWakeup::Timer => RecurrenceWakeupKind::Timer,
        CustomSourceRecurrenceWakeup::Backoff => RecurrenceWakeupKind::Backoff,
        CustomSourceRecurrenceWakeup::SourceEvent => RecurrenceWakeupKind::SourceEvent,
        CustomSourceRecurrenceWakeup::ProviderDefinedTrigger => {
            RecurrenceWakeupKind::ProviderDefinedTrigger
        }
    }
}

pub(crate) fn is_db_changed_trigger_projection(module: &Module, expr: ExprId) -> bool {
    matches!(
        &module.exprs()[expr].kind,
        ExprKind::Projection {
            base: ProjectionBase::Expr(_),
            path,
        } if path.segments().len() == 1 && path.segments().first().text() == "changed"
    )
}

pub(crate) fn type_argument_phrase(count: usize) -> String {
    format!("{count} type argument{}", if count == 1 { "" } else { "s" })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionTypeCheck {
    Match,
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionGenericConstructorRootCheck {
    Match(SourceOptionActualType),
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionTypeMismatch {
    pub(crate) span: SourceSpan,
    pub(crate) actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSourceOptionValue {
    pub(crate) field: crate::hir::RecordExprField,
    pub(crate) expected_surface: String,
    pub(crate) expected: SourceOptionExpectedType,
}

pub(crate) fn custom_source_contract_expected(
    module: &Module,
    annotation: TypeId,
    typing: &mut GateTypeContext<'_>,
) -> Option<(SourceOptionExpectedType, String)> {
    let expected = custom_source_contract_expected_type(module, annotation)?;
    let surface = typing.lower_annotation(annotation)?.to_string();
    Some((expected, surface))
}

pub(crate) fn custom_source_contract_expected_type(
    module: &Module,
    annotation: TypeId,
) -> Option<SourceOptionExpectedType> {
    SourceOptionExpectedType::from_hir_type(
        module,
        annotation,
        &HashMap::new(),
        SourceOptionTypeSurface::Contract,
    )
}
