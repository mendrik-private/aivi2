use crate::{
    CustomSourceArgumentSchema, CustomSourceCapabilityMember, CustomSourceContractMetadata, Item,
    Module, Name, TypeKind,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomSourceCapabilityKind {
    Operation,
    Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedCustomSourceCapabilityMember {
    pub provider_key: Box<str>,
    pub kind: CustomSourceCapabilityKind,
    pub member: CustomSourceCapabilityMember,
    pub binding_contract: CustomSourceContractMetadata,
}

pub(crate) fn resolve_custom_source_binding_contract(
    module: &Module,
    provider_key: &str,
) -> Option<CustomSourceContractMetadata> {
    resolve_exact_custom_source_contract(module, provider_key)
        .cloned()
        .or_else(|| {
            let (contract_key, member_name) = provider_key.rsplit_once('.')?;
            let resolved =
                resolve_custom_source_capability_member(module, contract_key, member_name)?;
            (resolved.kind == CustomSourceCapabilityKind::Operation)
                .then_some(resolved.binding_contract)
        })
}

pub(crate) fn resolve_custom_source_capability_member(
    module: &Module,
    contract_key: &str,
    member_name: &str,
) -> Option<ResolvedCustomSourceCapabilityMember> {
    let contract = resolve_exact_custom_source_contract(module, contract_key)?;
    let (kind, member) = if let Some(member) = contract
        .operations
        .iter()
        .find(|member| member.name.text() == member_name)
    {
        (CustomSourceCapabilityKind::Operation, member.clone())
    } else if let Some(member) = contract
        .commands
        .iter()
        .find(|member| member.name.text() == member_name)
    {
        (CustomSourceCapabilityKind::Command, member.clone())
    } else {
        return None;
    };
    Some(ResolvedCustomSourceCapabilityMember {
        provider_key: format!("{contract_key}.{}", member.name.text()).into_boxed_str(),
        kind,
        binding_contract: derive_custom_source_binding_contract(module, contract, &member),
        member,
    })
}

fn resolve_exact_custom_source_contract<'a>(
    module: &'a Module,
    provider_key: &str,
) -> Option<&'a CustomSourceContractMetadata> {
    let mut resolved = None;
    for (_, item) in module.items().iter() {
        let Item::SourceProviderContract(contract) = item else {
            continue;
        };
        if contract.provider.custom_key() != Some(provider_key) {
            continue;
        }
        if resolved.is_some() {
            return None;
        }
        resolved = Some(&contract.contract);
    }
    resolved
}

fn derive_custom_source_binding_contract(
    module: &Module,
    contract: &CustomSourceContractMetadata,
    member: &CustomSourceCapabilityMember,
) -> CustomSourceContractMetadata {
    let mut binding_contract = contract.clone();
    binding_contract
        .arguments
        .extend(capability_member_argument_schemas(module, member));
    binding_contract
}

fn capability_member_argument_schemas(
    module: &Module,
    member: &CustomSourceCapabilityMember,
) -> Vec<CustomSourceArgumentSchema> {
    let mut current = member.annotation;
    let mut index = 0usize;
    let mut arguments = Vec::new();
    while let TypeKind::Arrow { parameter, result } = &module.types()[current].kind {
        index += 1;
        let name = Name::new(format!("arg{index}"), member.span)
            .expect("compiler-generated capability argument names should be valid");
        arguments.push(CustomSourceArgumentSchema {
            span: member.span,
            name,
            annotation: *parameter,
        });
        current = *result;
    }
    arguments
}
