use std::collections::HashMap;

use crate::{
    BinaryOperator, DomainMemberHandle, DomainMemberKind, DomainMemberResolution, GateType, Item,
    Module, TypeParameterId, validate::GateTypeContext,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DomainBinaryOperatorMatch {
    pub(crate) callee: DomainMemberHandle,
    pub(crate) callee_type: GateType,
    pub(crate) result_type: GateType,
}

/// Selects the unique domain binary operator implementation for the given operand types.
///
/// Returns `Ok(Some(match))` when exactly one candidate matches, `Ok(None)` when no domain
/// operator is applicable (the caller should fall through to built-in arithmetic), and
/// `Err(candidates)` when more than one domain provides a matching operator — callers must
/// emit an [`crate::validate::GateIssue::AmbiguousDomainOperator`] diagnostic and recover.
pub(crate) fn select_domain_binary_operator(
    module: &Module,
    typing: &mut GateTypeContext<'_>,
    operator: BinaryOperator,
    left: &GateType,
    right: &GateType,
) -> Result<Option<DomainBinaryOperatorMatch>, Vec<DomainBinaryOperatorMatch>> {
    if !matches!(
        operator,
        BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
            | BinaryOperator::GreaterThan
            | BinaryOperator::LessThan
            | BinaryOperator::GreaterThanOrEqual
            | BinaryOperator::LessThanOrEqual
    ) {
        return Ok(None);
    }
    let mut matches = Vec::new();
    if let Some(result) = match_domain_binary_operator(module, typing, left, left, right, operator)
    {
        matches.push(result);
    }
    if !left.same_shape(right) {
        if let Some(result) =
            match_domain_binary_operator(module, typing, right, left, right, operator)
        {
            matches.push(result);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(matches),
    }
}

fn match_domain_binary_operator(
    module: &Module,
    typing: &mut GateTypeContext<'_>,
    domain_ty: &GateType,
    left: &GateType,
    right: &GateType,
    operator: BinaryOperator,
) -> Option<DomainBinaryOperatorMatch> {
    let GateType::Domain {
        item, arguments, ..
    } = domain_ty
    else {
        return None;
    };
    let Item::Domain(domain_item) = &module.items()[*item] else {
        return None;
    };
    let substitutions = domain_item
        .parameters
        .iter()
        .copied()
        .zip(arguments.iter().cloned())
        .collect::<HashMap<TypeParameterId, GateType>>();
    for (member_index, member) in domain_item.members.iter().enumerate() {
        if member.kind != DomainMemberKind::Operator
            || member.name.text() != binary_operator_text(operator)
        {
            continue;
        }
        let Some(callee_type) = typing.lower_hir_type(member.annotation, &substitutions) else {
            continue;
        };
        let GateType::Arrow {
            parameter: first,
            result: tail,
        } = &callee_type
        else {
            continue;
        };
        let GateType::Arrow {
            parameter: second,
            result,
        } = tail.as_ref()
        else {
            continue;
        };
        if first.as_ref().same_shape(left) && second.as_ref().same_shape(right) {
            let result_type = result.as_ref().clone();
            return Some(DomainBinaryOperatorMatch {
                callee: module.domain_member_handle(DomainMemberResolution {
                    domain: *item,
                    member_index,
                })?,
                callee_type,
                result_type,
            });
        }
    }
    None
}

pub(crate) fn binary_operator_text(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Modulo => "%",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::LessThan => "<",
        BinaryOperator::GreaterThanOrEqual => ">=",
        BinaryOperator::LessThanOrEqual => "<=",
        BinaryOperator::Equals => "==",
        BinaryOperator::NotEquals => "!=",
        BinaryOperator::And => "and",
        BinaryOperator::Or => "or",
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use crate::{
        ExprKind, Item, lower_module,
        validate::{GateExprEnv, GateTypeContext},
    };

    use super::*;

    fn lower_text(path: &str, text: &str) -> crate::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "domain-operator fixture should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn match_value_binary(path: &str, text: &str, value_name: &str) -> DomainBinaryOperatorMatch {
        let lowered = lower_text(path, text);
        assert!(
            !lowered.has_errors(),
            "domain-operator fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let (_, value) = module
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Value(item) if item.name.text() == value_name => {
                    Some((item.name.text(), item))
                }
                _ => None,
            })
            .expect("expected target value item");
        let ExprKind::Binary {
            left,
            operator,
            right,
        } = module.exprs()[value.body].kind.clone()
        else {
            panic!("expected `{value_name}` body to be a binary operator expression");
        };
        let mut typing = GateTypeContext::new(module);
        let env = GateExprEnv::default();
        let left_ty = typing
            .infer_expr(left, &env, None)
            .ty
            .expect("left operand should infer");
        let right_ty = typing
            .infer_expr(right, &env, None)
            .ty
            .expect("right operand should infer");
        select_domain_binary_operator(module, &mut typing, operator, &left_ty, &right_ty)
            .expect("domain operator selection should not be ambiguous in test fixtures")
            .expect("expected domain operator match")
    }

    #[test]
    fn selects_same_module_domain_operator_match() {
        let matched = match_value_binary(
            "domain-operator-duration.aivi",
            r#"
domain Duration over Int
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration

value total = 10ms + 5ms
"#,
            "total",
        );

        assert_eq!(matched.callee.domain_name.as_ref(), "Duration");
        assert_eq!(matched.callee.member_name.as_ref(), "+");
        assert!(matches!(matched.result_type, GateType::Domain { .. }));
    }

    #[test]
    fn selects_same_module_subtractive_domain_operator_match() {
        let matched = match_value_binary(
            "domain-operator-duration-subtract.aivi",
            r#"
domain Duration over Int
    literal ms : Int -> Duration
    (-) : Duration -> Duration -> Duration

value remaining = 10ms - 5ms
"#,
            "remaining",
        );

        assert_eq!(matched.callee.domain_name.as_ref(), "Duration");
        assert_eq!(matched.callee.member_name.as_ref(), "-");
        assert!(matches!(matched.result_type, GateType::Domain { .. }));
    }

    #[test]
    fn selects_parameterized_domain_operator_match() {
        let matched = match_value_binary(
            "domain-operator-amount.aivi",
            r#"
domain Amount A over A
    wrap : A -> Amount A
    (+) : Amount A -> Amount A -> Amount A

value total = wrap 1 + wrap 2
"#,
            "total",
        );

        assert_eq!(matched.callee.domain_name.as_ref(), "Amount");
        assert_eq!(matched.callee.member_name.as_ref(), "+");
        match matched.result_type {
            GateType::Domain { ref name, .. } => assert_eq!(name, "Amount"),
            other => panic!("expected parameterized domain result type, found {other:?}"),
        }
    }

    #[test]
    fn selects_same_module_path_join_domain_operator_match() {
        let matched = match_value_binary(
            "domain-operator-path.aivi",
            r#"
domain Path over Text
    root : Text -> Path
    (/) : Path -> Text -> Path

value nested = root "/tmp" / "config"
"#,
            "nested",
        );

        assert_eq!(matched.callee.domain_name.as_ref(), "Path");
        assert_eq!(matched.callee.member_name.as_ref(), "/");
        assert!(matches!(matched.result_type, GateType::Domain { .. }));
    }

    #[test]
    fn selects_same_module_multiplicative_domain_operator_match() {
        let matched = match_value_binary(
            "domain-operator-scale.aivi",
            r#"
domain Duration over Int
    literal ms : Int -> Duration
    (*) : Duration -> Int -> Duration

value scaled = 10ms * 2
"#,
            "scaled",
        );

        assert_eq!(matched.callee.domain_name.as_ref(), "Duration");
        assert_eq!(matched.callee.member_name.as_ref(), "*");
        assert!(matches!(matched.result_type, GateType::Domain { .. }));
    }
}
