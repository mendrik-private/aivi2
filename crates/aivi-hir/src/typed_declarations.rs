use aivi_base::SourceSpan;

use crate::{
    FunctionItem, GateType, Item, ItemId, Module, SignalItem, ValueItem,
    typecheck_context::{GateExprEnv, GateTypeContext},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypedDeclarationKind {
    Value,
    Function,
    Signal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedDeclarationInfo {
    pub item_id: ItemId,
    pub kind: TypedDeclarationKind,
    pub name: String,
    pub header_span: SourceSpan,
    pub name_span: SourceSpan,
    pub declared_type: Option<String>,
    pub inferred_type: Option<String>,
    pub annotation_matches_inferred: Option<bool>,
    pub has_explicit_constraints: bool,
}

pub fn collect_typed_declarations(module: &Module) -> Vec<TypedDeclarationInfo> {
    let mut typing = GateTypeContext::new(module);
    let mut declarations = Vec::new();

    for item_id in module.root_items().iter().copied() {
        let Some(info) = typed_declaration_info(module, item_id, &mut typing) else {
            continue;
        };
        declarations.push(info);
    }

    declarations
}

fn typed_declaration_info(
    module: &Module,
    item_id: ItemId,
    typing: &mut GateTypeContext<'_>,
) -> Option<TypedDeclarationInfo> {
    match &module.items()[item_id] {
        Item::Value(item) => Some(value_info(item_id, item, typing)),
        Item::Function(item) => Some(function_info(item_id, item, typing)),
        Item::Signal(item) => Some(signal_info(item_id, item, typing)),
        Item::Type(_)
        | Item::Class(_)
        | Item::Domain(_)
        | Item::SourceProviderContract(_)
        | Item::Instance(_)
        | Item::Use(_)
        | Item::Export(_)
        | Item::Hoist(_) => None,
    }
}

fn value_info(
    item_id: ItemId,
    item: &ValueItem,
    typing: &mut GateTypeContext<'_>,
) -> TypedDeclarationInfo {
    let declared = item.annotation.and_then(|annotation| typing.lower_annotation(annotation));
    let inferred = infer_value_type(item, typing);

    TypedDeclarationInfo {
        item_id,
        kind: TypedDeclarationKind::Value,
        name: item.name.text().to_owned(),
        header_span: item.header.span,
        name_span: item.name.span(),
        declared_type: declared.as_ref().map(ToString::to_string),
        inferred_type: inferred.as_ref().map(ToString::to_string),
        annotation_matches_inferred: type_match(declared.as_ref(), inferred.as_ref()),
        has_explicit_constraints: false,
    }
}

fn function_info(
    item_id: ItemId,
    item: &FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> TypedDeclarationInfo {
    let declared = declared_function_signature(item, typing);
    let inferred = infer_function_signature(item, typing);

    TypedDeclarationInfo {
        item_id,
        kind: TypedDeclarationKind::Function,
        name: item.name.text().to_owned(),
        header_span: item.header.span,
        name_span: item.name.span(),
        declared_type: declared.as_ref().map(|signature| signature.display.clone()),
        inferred_type: inferred.as_ref().map(|signature| signature.display.clone()),
        annotation_matches_inferred: type_match(
            declared.as_ref().map(|signature| &signature.comparable),
            inferred.as_ref().map(|signature| &signature.comparable),
        ),
        has_explicit_constraints: !item.context.is_empty(),
    }
}

fn signal_info(
    item_id: ItemId,
    item: &SignalItem,
    typing: &mut GateTypeContext<'_>,
) -> TypedDeclarationInfo {
    let declared = item.annotation.and_then(|annotation| typing.lower_annotation(annotation));
    let inferred = infer_signal_type(item, typing);

    TypedDeclarationInfo {
        item_id,
        kind: TypedDeclarationKind::Signal,
        name: item.name.text().to_owned(),
        header_span: item.header.span,
        name_span: item.name.span(),
        declared_type: declared.as_ref().map(ToString::to_string),
        inferred_type: inferred.as_ref().map(ToString::to_string),
        annotation_matches_inferred: type_match(declared.as_ref(), inferred.as_ref()),
        has_explicit_constraints: false,
    }
}

fn infer_value_type(item: &ValueItem, typing: &mut GateTypeContext<'_>) -> Option<GateType> {
    let info = typing.infer_expr(item.body, &GateExprEnv::default(), None);
    if !info.issues.is_empty() {
        return None;
    }
    info.actual_gate_type().or(info.ty)
}

fn infer_function_signature(
    item: &FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> Option<FunctionSignature> {
    let mut env = GateExprEnv::default();
    let mut parameters = Vec::with_capacity(item.parameters.len());
    for parameter in &item.parameters {
        let annotation = parameter.annotation?;
        let parameter_ty = typing.lower_open_annotation(annotation)?;
        env.locals.insert(parameter.binding, parameter_ty.clone());
        parameters.push(parameter_ty);
    }

    let info = typing.infer_expr(item.body, &env, None);
    if !info.issues.is_empty() {
        return None;
    }
    let result = info.actual_gate_type().or(info.ty)?;
    Some(FunctionSignature::new(Vec::new(), parameters, result))
}

fn infer_signal_type(item: &SignalItem, typing: &mut GateTypeContext<'_>) -> Option<GateType> {
    let body = item.body?;
    let info = typing.infer_expr(body, &GateExprEnv::default(), None);
    if !info.issues.is_empty() {
        return None;
    }
    let body_ty = info.actual_gate_type().or(info.ty)?;
    Some(if matches!(body_ty, GateType::Signal(_)) {
        body_ty
    } else {
        GateType::Signal(Box::new(body_ty))
    })
}

fn declared_function_signature(
    item: &FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> Option<FunctionSignature> {
    let constraints = item
        .context
        .iter()
        .filter_map(|constraint| typing.lower_open_annotation(*constraint))
        .collect::<Vec<_>>();
    let parameters = item
        .parameters
        .iter()
        .map(|parameter| parameter.annotation.and_then(|annotation| typing.lower_open_annotation(annotation)))
        .collect::<Option<Vec<_>>>()?;
    let result = item
        .annotation
        .and_then(|annotation| typing.lower_open_annotation(annotation))?;
    Some(FunctionSignature::new(constraints, parameters, result))
}

fn type_match(declared: Option<&GateType>, inferred: Option<&GateType>) -> Option<bool> {
    Some(declared?.same_shape(inferred?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FunctionSignature {
    display: String,
    comparable: GateType,
}

impl FunctionSignature {
    fn new(constraints: Vec<GateType>, parameters: Vec<GateType>, result: GateType) -> Self {
        let comparable = arrow_type(&parameters, result);
        let display = if constraints.is_empty() {
            comparable.to_string()
        } else {
            format!(
                "{} => {}",
                constraints
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                comparable
            )
        };
        Self {
            display,
            comparable,
        }
    }
}

fn arrow_type(parameters: &[GateType], result: GateType) -> GateType {
    let mut current = result;
    for parameter in parameters.iter().rev() {
        current = GateType::Arrow {
            parameter: Box::new(parameter.clone()),
            result: Box::new(current),
        };
    }
    current
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{TypedDeclarationKind, collect_typed_declarations};

    fn typed_declarations(input: &str) -> Vec<super::TypedDeclarationInfo> {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("typed-declarations.aivi", input.to_owned());
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "typed declaration test input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "typed declaration test input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        collect_typed_declarations(lowered.module())
    }

    #[test]
    fn matching_value_annotations_are_marked_as_matching() {
        let declarations = typed_declarations("value answer : Int = 42\n");
        let answer = declarations
            .iter()
            .find(|declaration| declaration.name == "answer")
            .expect("expected typed declaration for `answer`");

        assert_eq!(answer.kind, TypedDeclarationKind::Value);
        assert_eq!(answer.declared_type.as_deref(), Some("Int"));
        assert_eq!(answer.inferred_type.as_deref(), Some("Int"));
        assert_eq!(answer.annotation_matches_inferred, Some(true));
    }

    #[test]
    fn mismatched_value_annotations_are_marked_as_mismatching() {
        let declarations = typed_declarations("value answer : Text = 42\n");
        let answer = declarations
            .iter()
            .find(|declaration| declaration.name == "answer")
            .expect("expected typed declaration for `answer`");

        assert_eq!(answer.declared_type.as_deref(), Some("Text"));
        assert_eq!(answer.inferred_type.as_deref(), Some("Int"));
        assert_eq!(answer.annotation_matches_inferred, Some(false));
    }

    #[test]
    fn standalone_function_signatures_infer_against_normalized_parameter_types() {
        let declarations = typed_declarations(
            "type List Text -> List Text\n\
             func keepItems = items => items\n",
        );
        let keep_items = declarations
            .iter()
            .find(|declaration| declaration.name == "keepItems")
            .expect("expected typed declaration for `keepItems`");

        assert_eq!(keep_items.kind, TypedDeclarationKind::Function);
        assert_eq!(
            keep_items.declared_type.as_deref(),
            Some("List Text -> List Text")
        );
        assert_eq!(
            keep_items.inferred_type.as_deref(),
            Some("List Text -> List Text")
        );
        assert_eq!(keep_items.annotation_matches_inferred, Some(true));
    }

    #[test]
    fn unannotated_identity_functions_remain_uninferred() {
        let declarations = typed_declarations("func id = x => x\n");
        let id = declarations
            .iter()
            .find(|declaration| declaration.name == "id")
            .expect("expected typed declaration for `id`");

        assert_eq!(id.kind, TypedDeclarationKind::Function);
        assert_eq!(id.declared_type, None);
        assert_eq!(id.inferred_type, None);
        assert_eq!(id.annotation_matches_inferred, None);
    }

    #[test]
    fn constrained_function_signatures_preserve_constraint_display() {
        let declarations = typed_declarations(
            "type Eq A => A -> Bool\n\
             func isZero = value => value == 0\n",
        );
        let is_zero = declarations
            .iter()
            .find(|declaration| declaration.name == "isZero")
            .expect("expected typed declaration for `isZero`");

        assert_eq!(is_zero.declared_type.as_deref(), Some("A -> Bool"));
        assert!(is_zero.has_explicit_constraints);
    }
}
