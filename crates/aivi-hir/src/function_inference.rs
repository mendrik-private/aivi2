use std::collections::{HashMap, HashSet};

use crate::{
    GateType, Item, ItemId, Module,
    typecheck_context::{GateExprEnv, GateTypeContext},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionCallEvidence {
    pub(crate) item_id: ItemId,
    pub(crate) argument_types: Vec<GateType>,
    pub(crate) result_type: Option<GateType>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionSignatureEvidence {
    pub(crate) item_id: ItemId,
    pub(crate) parameter_types: Vec<GateType>,
    pub(crate) result_type: GateType,
}

#[derive(Clone, Debug, Default)]
struct InferenceSlot {
    ty: Option<GateType>,
    conflict: bool,
}

impl InferenceSlot {
    fn record(&mut self, candidate: GateType) -> bool {
        if self.conflict {
            return false;
        }
        match self.ty.as_ref() {
            None => {
                self.ty = Some(candidate);
                true
            }
            Some(existing) if existing.same_shape(&candidate) => false,
            Some(_) => {
                self.conflict = true;
                self.ty = None;
                true
            }
        }
    }

    fn value(&self) -> Option<GateType> {
        if self.conflict { None } else { self.ty.clone() }
    }
}

#[derive(Clone, Debug)]
struct FunctionInferenceState {
    parameter_slots: Vec<InferenceSlot>,
    result_slot: InferenceSlot,
}

impl FunctionInferenceState {
    fn from_function(
        function: &crate::hir::FunctionItem,
        typing: &mut GateTypeContext<'_>,
    ) -> Self {
        let parameter_slots = function
            .parameters
            .iter()
            .map(|parameter| InferenceSlot {
                ty: parameter
                    .annotation
                    .and_then(|annotation| typing.lower_open_annotation(annotation)),
                conflict: false,
            })
            .collect();
        let result_slot = InferenceSlot {
            ty: function
                .annotation
                .and_then(|annotation| typing.lower_open_annotation(annotation)),
            conflict: false,
        };
        Self {
            parameter_slots,
            result_slot,
        }
    }

    fn parameter_types(&self) -> Option<Vec<GateType>> {
        self.parameter_slots
            .iter()
            .map(InferenceSlot::value)
            .collect()
    }

    fn arrow_type(&self) -> Option<GateType> {
        let mut result = self.result_slot.value()?;
        for parameter in self.parameter_slots.iter().rev() {
            result = GateType::Arrow {
                parameter: Box::new(parameter.value()?),
                result: Box::new(result),
            };
        }
        Some(result)
    }

    fn record_call(&mut self, argument_types: &[GateType], result_type: Option<&GateType>) -> bool {
        let mut changed = false;
        for (slot, argument_ty) in self.parameter_slots.iter_mut().zip(argument_types.iter()) {
            changed |= slot.record(argument_ty.clone());
        }
        if let Some(result_ty) = result_type {
            changed |= self.result_slot.record(result_ty.clone());
        }
        changed
    }

    fn record_signature(&mut self, parameter_types: &[GateType], result_type: &GateType) -> bool {
        if parameter_types.len() != self.parameter_slots.len() {
            return false;
        }
        let mut changed = false;
        for (slot, parameter_ty) in self.parameter_slots.iter_mut().zip(parameter_types.iter()) {
            changed |= slot.record(parameter_ty.clone());
        }
        changed |= self.result_slot.record(result_type.clone());
        changed
    }
}

pub(crate) fn supports_same_module_function_inference(function: &crate::hir::FunctionItem) -> bool {
    function.type_parameters.is_empty() && function.context.is_empty()
}

pub(crate) fn infer_same_module_function_types(module: &Module) -> HashMap<ItemId, GateType> {
    let function_ids = module
        .items()
        .iter()
        .filter_map(|(item_id, item)| match item {
            Item::Function(function) if supports_same_module_function_inference(function) => {
                Some(item_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if function_ids.is_empty() {
        return HashMap::new();
    }

    let function_set = function_ids.iter().copied().collect::<HashSet<_>>();
    let mut seed_typing = GateTypeContext::new_for_function_inference(module);
    let mut states = function_ids
        .iter()
        .copied()
        .map(|item_id| {
            let Item::Function(function) = &module.items()[item_id] else {
                unreachable!("filtered above");
            };
            (
                item_id,
                FunctionInferenceState::from_function(function, &mut seed_typing),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut changed = true;
    while changed {
        changed = false;

        let seeded_item_types = states
            .iter()
            .filter_map(|(item_id, state)| state.arrow_type().map(|ty| (*item_id, ty)))
            .collect::<HashMap<_, _>>();

        let call_evidence = collect_call_evidence(module, &function_ids, seeded_item_types.clone());
        for evidence in call_evidence {
            let Some(state) = states.get_mut(&evidence.item_id) else {
                continue;
            };
            changed |= state.record_call(&evidence.argument_types, evidence.result_type.as_ref());
        }

        let contextual_evidence = collect_contextual_signature_evidence(
            module,
            &function_ids,
            seeded_item_types.clone(),
            &function_set,
        );
        for evidence in contextual_evidence {
            let Some(state) = states.get_mut(&evidence.item_id) else {
                continue;
            };
            changed |= state.record_signature(&evidence.parameter_types, &evidence.result_type);
        }

        changed |= infer_body_results(module, &mut states, seeded_item_types);
    }

    states
        .into_iter()
        .filter_map(|(item_id, state)| state.arrow_type().map(|ty| (item_id, ty)))
        .collect()
}

fn collect_call_evidence(
    module: &Module,
    function_ids: &[ItemId],
    seeded_item_types: HashMap<ItemId, GateType>,
) -> Vec<FunctionCallEvidence> {
    let mut typing =
        GateTypeContext::with_seeded_item_types(module, seeded_item_types.clone(), false);
    for item_id in function_ids {
        let Item::Function(function) = &module.items()[*item_id] else {
            continue;
        };
        let parameter_types = seeded_item_types
            .get(item_id)
            .and_then(|ty| parameter_types_from_signature(ty, function.parameters.len()));
        let mut env = GateExprEnv::default();
        for (index, parameter) in function.parameters.iter().enumerate() {
            let parameter_ty = parameter
                .annotation
                .and_then(|annotation| typing.lower_open_annotation(annotation))
                .or_else(|| {
                    parameter_types
                        .as_ref()
                        .and_then(|types| types.get(index).cloned())
                });
            if let Some(parameter_ty) = parameter_ty {
                env.locals.insert(parameter.binding, parameter_ty);
            }
        }
        let _ = typing.infer_expr(function.body, &env, None);
    }
    let mut evidence = typing.take_function_call_evidence();
    evidence.extend(
        typing
            .take_function_signature_evidence()
            .into_iter()
            .filter(|evidence| {
                !evidence.result_type.has_type_params()
                    && evidence
                        .parameter_types
                        .iter()
                        .all(|parameter| !parameter.has_type_params())
            })
            .map(|evidence| FunctionCallEvidence {
                item_id: evidence.item_id,
                argument_types: evidence.parameter_types,
                result_type: Some(evidence.result_type),
            }),
    );
    evidence
}

fn collect_contextual_signature_evidence(
    module: &Module,
    function_ids: &[ItemId],
    seeded_item_types: HashMap<ItemId, GateType>,
    function_set: &HashSet<ItemId>,
) -> Vec<FunctionSignatureEvidence> {
    let typing = GateTypeContext::with_seeded_item_types(module, seeded_item_types, false);
    crate::typecheck::collect_contextual_function_signature_evidence(
        module,
        function_ids,
        typing,
        function_set,
    )
}

fn parameter_types_from_signature(ty: &GateType, arity: usize) -> Option<Vec<GateType>> {
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

fn infer_body_results(
    module: &Module,
    states: &mut HashMap<ItemId, FunctionInferenceState>,
    seeded_item_types: HashMap<ItemId, GateType>,
) -> bool {
    let mut typing = GateTypeContext::with_seeded_item_types(module, seeded_item_types, false);
    let mut changed = false;
    for (item_id, state) in states.iter_mut() {
        if state.result_slot.value().is_some() {
            continue;
        }
        let Some(parameter_types) = state.parameter_types() else {
            continue;
        };
        let Item::Function(function) = &module.items()[*item_id] else {
            continue;
        };
        let mut env = GateExprEnv::default();
        for (parameter, parameter_ty) in function.parameters.iter().zip(parameter_types.iter()) {
            env.locals.insert(parameter.binding, parameter_ty.clone());
        }
        let body_info = typing.infer_expr(function.body, &env, None);
        let Some(result_ty) = body_info.actual_gate_type().or(body_info.ty) else {
            continue;
        };
        changed |= state.result_slot.record(result_ty);
    }
    changed
}
