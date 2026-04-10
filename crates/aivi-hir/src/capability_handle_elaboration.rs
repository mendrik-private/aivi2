use std::collections::BTreeMap;

use aivi_base::{Diagnostic, DiagnosticCode, DiagnosticLabel, SourceSpan};
use aivi_typing::BuiltinSourceProvider;

use crate::{
    BuiltinType, CustomCapabilityCommandSpec, CustomSourceOptionSchema, Decorator,
    DecoratorPayload, Expr, ExprId, ExprKind, ImportBinding, ImportBindingMetadata,
    ImportBindingResolution, ImportId, ImportValueType, IntrinsicValue, Item, ItemId, Module, Name,
    NamePath, NonEmpty, ProjectionBase, RecordExpr, ResolutionState, SignalItem, SourceDecorator,
    SourceProviderRef, TermReference, TermResolution, TypeKind, TypeResolution, ValueItem,
    custom_source_capabilities::{
        CustomSourceCapabilityKind, resolve_custom_source_capability_member,
    },
};

pub(crate) fn is_builtin_source_capability_family_path(path: &NamePath) -> bool {
    if path.segments().len() != 1 {
        return false;
    }
    matches!(
        path.segments().first().text(),
        "fs" | "http"
            | "db"
            | "env"
            | "log"
            | "stdio"
            | "random"
            | "process"
            | "path"
            | "dbus"
            | "imap"
            | "smtp"
            | "time"
            | "api"
    )
}

pub(crate) fn elaborate_capability_handles(module: &mut Module, diagnostics: &mut Vec<Diagnostic>) {
    let handles = collect_capability_handles(module, diagnostics);
    rewrite_capability_uses(module, &handles, diagnostics);
}

#[derive(Clone, Debug)]
struct CapabilityHandleBinding {
    span: SourceSpan,
    provider: CapabilityHandleProvider,
    arguments: Vec<ExprId>,
    options: Option<ExprId>,
}

#[derive(Clone, Debug)]
enum CapabilityHandleProvider {
    BuiltinFamily(BuiltinCapabilityFamily),
    Custom(Box<str>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinCapabilityFamily {
    Fs,
    Http,
    Db,
    Env,
    Log,
    Stdio,
    Random,
    Process,
    Path,
    Dbus,
    Imap,
    Smtp,
    Time,
    Api,
}

#[derive(Clone, Debug)]
struct CapabilityInvocation {
    span: SourceSpan,
    handle: ItemId,
    member: String,
    arguments: Vec<ExprId>,
}

#[derive(Clone, Debug)]
struct CapturedCustomCommandOption {
    name: Name,
    annotation: crate::TypeId,
    value: ExprId,
}

fn collect_capability_handles(
    module: &mut Module,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<ItemId, CapabilityHandleBinding> {
    let item_ids = module
        .items()
        .iter()
        .map(|(item_id, _)| item_id)
        .collect::<Vec<_>>();
    let mut handles = BTreeMap::new();
    for item_id in item_ids {
        let Some((signal, source)) = signal_with_source_decorator(module, item_id) else {
            continue;
        };
        let Some(provider_path) = source.provider.as_ref() else {
            continue;
        };
        let annotation_is_signal = signal_annotation_is_signal(module, signal);
        let classification = classify_handle_provider(provider_path);
        let is_candidate =
            signal.body.is_none() && signal.reactive_updates.is_empty() && !annotation_is_signal;
        match classification {
            HandleProviderClassification::BuiltinFamily(family) if is_candidate => {
                if signal.annotation.is_none() {
                    diagnostics.push(
                        Diagnostic::error(
                            "source capability handles require an explicit handle type annotation",
                        )
                        .with_code(code("missing-source-capability-annotation"))
                        .with_label(DiagnosticLabel::primary(
                            signal.header.span,
                            "declare a nominal handle type such as `FsSource` here",
                        )),
                    );
                }
                handles.insert(
                    item_id,
                    CapabilityHandleBinding {
                        span: signal.header.span,
                        provider: CapabilityHandleProvider::BuiltinFamily(family),
                        arguments: source.arguments.clone(),
                        options: source.options,
                    },
                );
            }
            HandleProviderClassification::Custom(key) if is_candidate => {
                if signal.annotation.is_none() {
                    diagnostics.push(
                        Diagnostic::error(
                            "source capability handles require an explicit handle type annotation",
                        )
                        .with_code(code("missing-source-capability-annotation"))
                        .with_label(DiagnosticLabel::primary(
                            signal.header.span,
                            "declare a nominal handle type for this provider capability",
                        )),
                    );
                }
                handles.insert(
                    item_id,
                    CapabilityHandleBinding {
                        span: signal.header.span,
                        provider: CapabilityHandleProvider::Custom(key),
                        arguments: source.arguments.clone(),
                        options: source.options,
                    },
                );
            }
            HandleProviderClassification::BuiltinFamily(_) => diagnostics.push(
                Diagnostic::error(
                    "source decorators must name a provider variant such as `timer.every`",
                )
                .with_code(code("invalid-source-provider"))
                .with_label(DiagnosticLabel::primary(
                    provider_path.span(),
                    "provider families such as `fs` are only valid on bodyless source capability handles",
                )),
            ),
            HandleProviderClassification::BuiltinProvider(provider) if is_candidate => diagnostics.push(
                Diagnostic::error(
                    "source capability handles must name a provider family such as `fs`, not a concrete provider variant",
                )
                .with_code(code("invalid-source-capability-provider"))
                .with_label(DiagnosticLabel::primary(
                    provider_path.span(),
                    format!("`{}` is already a concrete provider binding", provider.key()),
                )),
            ),
            HandleProviderClassification::BuiltinProvider(_)
            | HandleProviderClassification::Custom(_)
            | HandleProviderClassification::InvalidShape => {}
        }
    }
    for item_id in handles.keys().copied().collect::<Vec<_>>() {
        let Some(Item::Signal(signal)) = module.arenas.items.get_mut(item_id) else {
            continue;
        };
        signal.is_source_capability_handle = true;
    }
    handles
}

fn signal_annotation_is_signal(module: &Module, signal: &SignalItem) -> bool {
    let Some(annotation) = signal.annotation else {
        return false;
    };
    type_annotation_is_signal(module, annotation)
}

fn type_annotation_is_signal(module: &Module, type_id: crate::TypeId) -> bool {
    match &module.types()[type_id].kind {
        TypeKind::Apply { callee, .. } => type_reference_is_signal(module, *callee),
        TypeKind::Name(_) => type_reference_is_signal(module, type_id),
        _ => false,
    }
}

fn type_reference_is_signal(module: &Module, type_id: crate::TypeId) -> bool {
    let TypeKind::Name(reference) = &module.types()[type_id].kind else {
        return false;
    };
    matches!(
        reference.resolution,
        ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal))
    ) || reference.path.segments().last().text() == "Signal"
}

fn rewrite_capability_uses(
    module: &mut Module,
    handles: &BTreeMap<ItemId, CapabilityHandleBinding>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let item_ids = module
        .items()
        .iter()
        .map(|(item_id, _)| item_id)
        .collect::<Vec<_>>();
    let mut signal_rewrites = Vec::<SignalCapabilityRewrite>::new();
    let mut value_rewrites = Vec::<ValueCapabilityRewrite>::new();
    for item_id in item_ids {
        match module.items()[item_id].clone() {
            Item::Signal(signal) if !signal.is_source_capability_handle => {
                let Some(body) = signal.body else {
                    continue;
                };
                let Some(invocation) = parse_capability_invocation(module, body, handles) else {
                    continue;
                };
                let Some(handle) = handles.get(&invocation.handle) else {
                    continue;
                };
                if signal_has_source_decorator(module, &signal) {
                    diagnostics.push(
                        Diagnostic::error(
                            "signals cannot mix a direct capability operation body with an explicit `@source` decorator",
                        )
                        .with_code(code("conflicting-source-capability-binding"))
                        .with_label(DiagnosticLabel::primary(
                            signal.header.span,
                            "choose either an explicit provider variant or a capability handle operation",
                        )),
                    );
                    continue;
                }
                if let Some(rewrite) =
                    lower_signal_capability_use(module, &signal, handle, &invocation, diagnostics)
                {
                    signal_rewrites.push(SignalCapabilityRewrite { item_id, rewrite })
                }
            }
            Item::Value(value) => {
                let Some(invocation) = parse_capability_invocation(module, value.body, handles)
                else {
                    continue;
                };
                let Some(handle) = handles.get(&invocation.handle) else {
                    continue;
                };
                if let Some(body) =
                    lower_value_capability_use(module, &value, handle, &invocation, diagnostics)
                {
                    value_rewrites.push(ValueCapabilityRewrite { item_id, body })
                }
            }
            _ => {}
        }
    }
    for rewrite in signal_rewrites {
        apply_signal_rewrite(module, rewrite);
    }
    for rewrite in value_rewrites {
        apply_value_rewrite(module, rewrite);
    }
}

fn parse_capability_invocation(
    module: &Module,
    expr_id: ExprId,
    handles: &BTreeMap<ItemId, CapabilityHandleBinding>,
) -> Option<CapabilityInvocation> {
    let expr = &module.exprs()[expr_id];
    match &expr.kind {
        ExprKind::Apply { callee, arguments } => {
            let (handle, member) = projection_capability_target(module, *callee, handles)?;
            Some(CapabilityInvocation {
                span: expr.span,
                handle,
                member,
                arguments: arguments.iter().copied().collect(),
            })
        }
        ExprKind::Projection { .. } => {
            let (handle, member) = projection_capability_target(module, expr_id, handles)?;
            Some(CapabilityInvocation {
                span: expr.span,
                handle,
                member,
                arguments: Vec::new(),
            })
        }
        _ => None,
    }
}

fn projection_capability_target(
    module: &Module,
    expr_id: ExprId,
    handles: &BTreeMap<ItemId, CapabilityHandleBinding>,
) -> Option<(ItemId, String)> {
    let ExprKind::Projection { base, path } = &module.exprs()[expr_id].kind else {
        return None;
    };
    if path.segments().len() != 1 {
        return None;
    }
    let ProjectionBase::Expr(base_expr) = base else {
        return None;
    };
    let ExprKind::Name(reference) = &module.exprs()[*base_expr].kind else {
        return None;
    };
    let ResolutionState::Resolved(TermResolution::Item(item_id)) = reference.resolution else {
        return None;
    };
    handles
        .contains_key(&item_id)
        .then(|| (item_id, path.segments().first().text().to_owned()))
}

fn lower_signal_capability_use(
    module: &mut Module,
    signal: &SignalItem,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SignalRewritePlan> {
    if signal.annotation.is_none() {
        diagnostics.push(
            Diagnostic::error(
                "source capability operations need an explicit `Signal ...` annotation on the target signal",
            )
            .with_code(code("missing-source-capability-signal-annotation"))
            .with_label(DiagnosticLabel::primary(
                signal.header.span,
                "annotate the signal with its decoded payload type before using a capability operation",
            )),
        );
    }
    match &handle.provider {
        CapabilityHandleProvider::BuiltinFamily(family) => {
            let provider = match lower_builtin_signal_member(
                module,
                *family,
                handle,
                invocation,
                diagnostics,
            ) {
                Some(provider) => provider,
                None => {
                    if supports_builtin_value_member(*family, invocation.member.as_str()) {
                        diagnostics.push(
                            Diagnostic::error(format!(
                                "capability member `{}` is a one-shot command or query; use `value` instead of `signal`",
                                invocation.member
                            ))
                            .with_code(code("invalid-source-capability-signal-member"))
                            .with_label(DiagnosticLabel::primary(
                                invocation.span,
                                "lower this capability use into a top-level value",
                            )),
                        );
                    }
                    return None;
                }
            };
            Some(SignalRewritePlan {
                decorator: synthesize_source_decorator(invocation.span, provider),
            })
        }
        CapabilityHandleProvider::Custom(_) => {
            let CapabilityHandleProvider::Custom(key) = &handle.provider else {
                unreachable!("custom capability lowering should preserve its provider kind");
            };
            let Some(resolved) = resolve_custom_source_capability_member(
                module,
                key.as_ref(),
                invocation.member.as_str(),
            ) else {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "unknown capability member `{}` for custom provider `{key}`",
                        invocation.member
                    ))
                    .with_code(code("unknown-source-capability-member"))
                    .with_label(DiagnosticLabel::primary(
                        invocation.span,
                        "declare this capability member on the provider contract before using it through the handle",
                    )),
                );
                return None;
            };
            match resolved.kind {
                CustomSourceCapabilityKind::Operation => Some(SignalRewritePlan {
                    decorator: synthesize_source_decorator(
                        invocation.span,
                        SourceDecorator {
                            provider: Some(provider_key_name_path(
                                invocation.span,
                                resolved.provider_key.as_ref(),
                            )),
                            arguments: inherited_arguments(handle, &invocation.arguments),
                            options: handle.options,
                        },
                    ),
                }),
                CustomSourceCapabilityKind::Command => {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "capability member `{}` is a one-shot custom command; direct custom command execution through handle values is not implemented yet",
                            invocation.member
                        ))
                        .with_code(code("invalid-source-capability-signal-member"))
                        .with_label(DiagnosticLabel::primary(
                            invocation.span,
                            "bind custom source operations as `signal`; custom commands still need an explicit runtime path",
                        )),
                    );
                    None
                }
            }
        }
    }
}

fn lower_value_capability_use(
    module: &mut Module,
    _value: &ValueItem,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    match &handle.provider {
        CapabilityHandleProvider::BuiltinFamily(family) => {
            if let Some(expr) =
                lower_builtin_value_member(module, *family, handle, invocation, diagnostics)
            {
                return Some(expr);
            }
            if supports_builtin_signal_member(*family, invocation.member.as_str()) {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "capability member `{}` is a reactive source operation; use `signal` instead of `value`",
                        invocation.member
                    ))
                    .with_code(code("invalid-source-capability-value-member"))
                    .with_label(DiagnosticLabel::primary(
                        invocation.span,
                        "bind the result as a `signal` so the source lifecycle can own it",
                    )),
                );
                return None;
            }
            diagnostics.push(
                Diagnostic::error(format!(
                    "unknown capability member `{}` for this provider family",
                    invocation.member
                ))
                .with_code(code("unknown-source-capability-member"))
                .with_label(DiagnosticLabel::primary(
                    invocation.span,
                    "no built-in capability lowering is defined for this member",
                )),
            );
            None
        }
        CapabilityHandleProvider::Custom(key) => {
            let Some(resolved) = resolve_custom_source_capability_member(
                module,
                key.as_ref(),
                invocation.member.as_str(),
            ) else {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "unknown capability member `{}` for custom provider `{key}`",
                        invocation.member
                    ))
                    .with_code(code("unknown-source-capability-member"))
                    .with_label(DiagnosticLabel::primary(
                        invocation.span,
                        "declare this capability member on the provider contract before using it through the handle",
                    )),
                );
                return None;
            };
            match resolved.kind {
                CustomSourceCapabilityKind::Operation => {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "capability member `{}` is a reactive source operation; use `signal` instead of `value`",
                            invocation.member
                        ))
                        .with_code(code("invalid-source-capability-value-member"))
                        .with_label(DiagnosticLabel::primary(
                            invocation.span,
                            "bind the result as a `signal` so the source lifecycle can own it",
                        )),
                    );
                    None
                }
                CustomSourceCapabilityKind::Command => {
                    lower_custom_value_member(module, handle, invocation, &resolved, diagnostics)
                }
            }
        }
    }
}

fn lower_custom_value_member(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    resolved: &crate::custom_source_capabilities::ResolvedCustomSourceCapabilityMember,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    let captured_options =
        captured_custom_command_options(module, handle, resolved.option_schemas.as_slice());
    let ty = custom_command_import_type(
        module,
        invocation,
        resolved,
        captured_options.as_slice(),
        diagnostics,
    )?;
    let spec = CustomCapabilityCommandSpec {
        provider_key: resolved.contract_key.clone(),
        command: resolved.member.name.text().into(),
        provider_arguments: resolved.binding_contract.arguments[..resolved.provider_argument_count]
            .iter()
            .map(|argument| argument.name.text().into())
            .collect(),
        options: captured_options
            .iter()
            .map(|option| option.name.text().into())
            .collect(),
        arguments: resolved
            .member_argument_schemas
            .iter()
            .map(|argument| argument.name.text().into())
            .collect(),
    };
    let import = alloc_custom_command_import(module, invocation.span, spec, ty);
    let mut arguments = handle.arguments.clone();
    arguments.extend(captured_options.iter().map(|option| option.value));
    arguments.extend(invocation.arguments.iter().copied());
    Some(build_import_call(
        module,
        import,
        invocation.span,
        arguments,
    ))
}

fn captured_custom_command_options(
    module: &Module,
    handle: &CapabilityHandleBinding,
    option_schemas: &[CustomSourceOptionSchema],
) -> Vec<CapturedCustomCommandOption> {
    let Some(options) = handle.options else {
        return Vec::new();
    };
    let ExprKind::Record(RecordExpr { fields }) = &module.exprs()[options].kind else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|field| {
            let schema = option_schemas
                .iter()
                .find(|schema| schema.name.text() == field.label.text())?;
            Some(CapturedCustomCommandOption {
                name: field.label.clone(),
                annotation: schema.annotation,
                value: field.value,
            })
        })
        .collect()
}

fn custom_command_import_type(
    module: &Module,
    invocation: &CapabilityInvocation,
    resolved: &crate::custom_source_capabilities::ResolvedCustomSourceCapabilityMember,
    captured_options: &[CapturedCustomCommandOption],
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ImportValueType> {
    let Some(mut ty) = crate::exports::import_value_type(module, resolved.member.annotation) else {
        diagnostics.push(
            Diagnostic::error(format!(
                "capability member `{}` uses a command type that cannot lower through the shared task runtime",
                invocation.member
            ))
            .with_code(code("unsupported-custom-source-capability"))
            .with_label(DiagnosticLabel::primary(
                invocation.span,
                "rewrite this command to use only closed runtime-lowerable argument and task result types",
            )),
        );
        return None;
    };
    for option in captured_options.iter().rev() {
        let Some(parameter) = crate::exports::import_value_type(module, option.annotation) else {
            diagnostics.push(
                Diagnostic::error(format!(
                    "source handle option `{}` cannot flow through the shared custom command runtime",
                    option.name.text()
                ))
                .with_code(code("unsupported-custom-source-capability"))
                .with_label(DiagnosticLabel::primary(
                    invocation.span,
                    "use only closed runtime-lowerable option types on custom command handles",
                )),
            );
            return None;
        };
        ty = ImportValueType::Arrow {
            parameter: Box::new(parameter),
            result: Box::new(ty),
        };
    }
    for argument in resolved.binding_contract.arguments[..resolved.provider_argument_count]
        .iter()
        .rev()
    {
        let Some(parameter) = crate::exports::import_value_type(module, argument.annotation) else {
            diagnostics.push(
                Diagnostic::error(format!(
                    "custom provider argument `{}` cannot lower through handle command values yet",
                    argument.name.text()
                ))
                .with_code(code("unsupported-custom-source-capability"))
                .with_label(DiagnosticLabel::primary(
                    invocation.span,
                    "use only closed runtime-lowerable provider argument types on custom command handles",
                )),
            );
            return None;
        };
        ty = ImportValueType::Arrow {
            parameter: Box::new(parameter),
            result: Box::new(ty),
        };
    }
    Some(ty)
}

fn alloc_custom_command_import(
    module: &mut Module,
    span: SourceSpan,
    spec: CustomCapabilityCommandSpec,
    ty: ImportValueType,
) -> ImportId {
    let index = module.imports().len();
    let local_name = Name::new(format!("customCapabilityCommand{index}"), span)
        .expect("compiler-generated custom command import names should stay valid");
    module
        .alloc_import(ImportBinding {
            span,
            source_module: None,
            imported_name: local_name.clone(),
            local_name,
            resolution: ImportBindingResolution::Resolved,
            metadata: ImportBindingMetadata::IntrinsicValue {
                value: IntrinsicValue::CustomCapabilityCommand(Box::leak(Box::new(spec))),
                ty,
            },
            callable_type: None,
            deprecation: None,
        })
        .expect("capability lowering should fit inside the import arena")
}

fn build_import_call(
    module: &mut Module,
    import: ImportId,
    span: SourceSpan,
    arguments: Vec<ExprId>,
) -> ExprId {
    let local_name = module.imports()[import].local_name.clone();
    let path = NamePath::from_vec(vec![local_name])
        .expect("compiler-generated custom command import paths should stay valid");
    let callee = module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                path,
                TermResolution::Import(import),
            )),
        })
        .expect("capability lowering should fit inside the expression arena");
    if arguments.is_empty() {
        return callee;
    }
    let arguments = NonEmpty::from_vec(arguments)
        .expect("custom capability command applications always pass at least one argument");
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Apply { callee, arguments },
        })
        .expect("capability lowering should fit inside the expression arena")
}

fn lower_builtin_signal_member(
    module: &mut Module,
    family: BuiltinCapabilityFamily,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SourceDecorator> {
    match family {
        BuiltinCapabilityFamily::Fs => match invocation.member.as_str() {
            "read" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::FsRead,
                )),
                arguments: vec![scoped_path_argument(
                    module,
                    handle,
                    invocation.arguments.first().copied(),
                    invocation.span,
                    diagnostics,
                )?],
                options: handle.options,
            }),
            "watch" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::FsWatch,
                )),
                arguments: vec![scoped_path_argument(
                    module,
                    handle,
                    invocation.arguments.first().copied(),
                    invocation.span,
                    diagnostics,
                )?],
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Http => match invocation.member.as_str() {
            "get" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::HttpGet,
                )),
                arguments: vec![scoped_http_url_argument(
                    module,
                    handle,
                    invocation.arguments.first().copied(),
                    invocation.span,
                    diagnostics,
                )?],
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Db => match invocation.member.as_str() {
            "connect" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::DbConnect,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            "live" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::DbLive,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Env => match invocation.member.as_str() {
            "get" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::EnvGet,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Stdio => match invocation.member.as_str() {
            "read" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::StdioRead,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Process => match invocation.member.as_str() {
            "spawn" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::ProcessSpawn,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            "args" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::ProcessArgs,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            "cwd" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::ProcessCwd,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Path => {
            let provider = match invocation.member.as_str() {
                "home" => BuiltinSourceProvider::PathHome,
                "configHome" => BuiltinSourceProvider::PathConfigHome,
                "dataHome" => BuiltinSourceProvider::PathDataHome,
                "cacheHome" => BuiltinSourceProvider::PathCacheHome,
                "tempDir" => BuiltinSourceProvider::PathTempDir,
                _ => return None,
            };
            Some(SourceDecorator {
                provider: Some(provider_name_path(invocation.span, provider)),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            })
        }
        BuiltinCapabilityFamily::Dbus => {
            let provider = match invocation.member.as_str() {
                "ownName" => BuiltinSourceProvider::DbusOwnName,
                "signal" => BuiltinSourceProvider::DbusSignal,
                "method" => BuiltinSourceProvider::DbusMethod,
                _ => return None,
            };
            Some(SourceDecorator {
                provider: Some(provider_name_path(invocation.span, provider)),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            })
        }
        BuiltinCapabilityFamily::Log
        | BuiltinCapabilityFamily::Random
        | BuiltinCapabilityFamily::Smtp => None,
        BuiltinCapabilityFamily::Imap => {
            let provider = match invocation.member.as_str() {
                "connect" => BuiltinSourceProvider::ImapConnect,
                "idle" => BuiltinSourceProvider::ImapIdle,
                "fetchBody" => BuiltinSourceProvider::ImapFetchBody,
                _ => return None,
            };
            Some(SourceDecorator {
                provider: Some(provider_name_path(invocation.span, provider)),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            })
        }
        BuiltinCapabilityFamily::Time => match invocation.member.as_str() {
            "nowMs" => Some(SourceDecorator {
                provider: Some(provider_name_path(
                    invocation.span,
                    BuiltinSourceProvider::TimeNowMs,
                )),
                arguments: inherited_arguments(handle, &invocation.arguments),
                options: handle.options,
            }),
            _ => None,
        },
        BuiltinCapabilityFamily::Api => {
            lower_api_signal_member(module, handle, invocation, diagnostics)
        }
    }
}

fn lower_builtin_value_member(
    module: &mut Module,
    family: BuiltinCapabilityFamily,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    match family {
        BuiltinCapabilityFamily::Fs => {
            lower_fs_value_member(module, handle, invocation, diagnostics)
        }
        BuiltinCapabilityFamily::Http => {
            let intrinsic = match invocation.member.as_str() {
                "get" => IntrinsicValue::HttpGet,
                "getBytes" => IntrinsicValue::HttpGetBytes,
                "getStatus" => IntrinsicValue::HttpGetStatus,
                "post" => IntrinsicValue::HttpPost,
                "put" => IntrinsicValue::HttpPut,
                "delete" => IntrinsicValue::HttpDelete,
                "head" => IntrinsicValue::HttpHead,
                "postJson" => IntrinsicValue::HttpPostJson,
                _ => return None,
            };
            let arguments = combine_http_value_arguments(
                module,
                handle,
                &invocation.arguments,
                invocation.span,
                diagnostics,
            );
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                arguments,
            ))
        }
        BuiltinCapabilityFamily::Db => {
            let intrinsic = match invocation.member.as_str() {
                "query" => IntrinsicValue::DbQuery,
                "commit" | "exec" => IntrinsicValue::DbCommit,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Env => {
            let intrinsic = match invocation.member.as_str() {
                "get" => IntrinsicValue::EnvGet,
                "list" => IntrinsicValue::EnvList,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Log => {
            let intrinsic = match invocation.member.as_str() {
                "emit" => IntrinsicValue::LogEmit,
                "emitContext" => IntrinsicValue::LogEmitContext,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Stdio => {
            let intrinsic = match invocation.member.as_str() {
                "write" | "stdoutWrite" => IntrinsicValue::StdoutWrite,
                "writeError" | "stderrWrite" => IntrinsicValue::StderrWrite,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Random => {
            let intrinsic = match invocation.member.as_str() {
                "randomInt" | "int" => IntrinsicValue::RandomInt,
                "randomBytes" | "bytes" => IntrinsicValue::RandomBytes,
                "randomFloat" | "float" => IntrinsicValue::RandomFloat,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Path => {
            let intrinsic = match invocation.member.as_str() {
                "dataHome" => IntrinsicValue::XdgDataHome,
                "configHome" => IntrinsicValue::XdgConfigHome,
                "cacheHome" => IntrinsicValue::XdgCacheHome,
                "stateHome" => IntrinsicValue::XdgStateHome,
                "runtimeDir" => IntrinsicValue::XdgRuntimeDir,
                "dataDirs" => IntrinsicValue::XdgDataDirs,
                "configDirs" => IntrinsicValue::XdgConfigDirs,
                _ => return None,
            };
            Some(build_intrinsic_call(
                module,
                intrinsic,
                invocation.span,
                inherited_arguments(handle, &invocation.arguments),
            ))
        }
        BuiltinCapabilityFamily::Process
        | BuiltinCapabilityFamily::Dbus
        | BuiltinCapabilityFamily::Imap
        | BuiltinCapabilityFamily::Time => None,
        BuiltinCapabilityFamily::Smtp => None,
        BuiltinCapabilityFamily::Api => {
            lower_api_value_member(module, handle, invocation, diagnostics)
        }
    }
}

fn lower_fs_value_member(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    match invocation.member.as_str() {
        "read" | "readText" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsReadText,
                invocation.span,
                vec![path],
            ))
        }
        "readBytes" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsReadBytes,
                invocation.span,
                vec![path],
            ))
        }
        "readDir" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsReadDir,
                invocation.span,
                vec![path],
            ))
        }
        "exists" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsExists,
                invocation.span,
                vec![path],
            ))
        }
        "write" | "writeText" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            let text = *invocation.arguments.get(1)?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsWriteText,
                invocation.span,
                vec![path, text],
            ))
        }
        "writeBytes" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            let bytes = *invocation.arguments.get(1)?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsWriteBytes,
                invocation.span,
                vec![path, bytes],
            ))
        }
        "createDirAll" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsCreateDirAll,
                invocation.span,
                vec![path],
            ))
        }
        "delete" | "deleteFile" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsDeleteFile,
                invocation.span,
                vec![path],
            ))
        }
        "deleteDir" => {
            let path = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsDeleteDir,
                invocation.span,
                vec![path],
            ))
        }
        "rename" | "move" => {
            let from = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            let to = scoped_path_argument(
                module,
                handle,
                invocation.arguments.get(1).copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsRename,
                invocation.span,
                vec![from, to],
            ))
        }
        "copy" => {
            let from = scoped_path_argument(
                module,
                handle,
                invocation.arguments.first().copied(),
                invocation.span,
                diagnostics,
            )?;
            let to = scoped_path_argument(
                module,
                handle,
                invocation.arguments.get(1).copied(),
                invocation.span,
                diagnostics,
            )?;
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::FsCopy,
                invocation.span,
                vec![from, to],
            ))
        }
        _ => None,
    }
}

fn inherited_arguments(
    handle: &CapabilityHandleBinding,
    member_arguments: &[ExprId],
) -> Vec<ExprId> {
    let mut arguments = handle.arguments.clone();
    arguments.extend(member_arguments.iter().copied());
    arguments
}

fn try_get_plain_text_literal(module: &Module, expr_id: ExprId) -> Option<&str> {
    let expr = module.arenas.exprs.get(expr_id)?;
    match &expr.kind {
        ExprKind::Text(lit) if !lit.has_interpolation() => match lit.segments.as_slice() {
            [] => Some(""),
            [crate::TextSegment::Text(frag)] => Some(&frag.raw),
            _ => None,
        },
        _ => None,
    }
}

fn synthesize_text_literal(module: &mut Module, text: &str, span: SourceSpan) -> ExprId {
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Text(crate::TextLiteral {
                segments: vec![crate::TextSegment::Text(crate::TextFragment {
                    raw: text.into(),
                    span,
                })],
            }),
        })
        .expect("api capability lowering should fit inside the expression arena")
}

fn lower_api_signal_member(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SourceDecorator> {
    let operation_id = invocation.member.clone();
    let spec_path_expr = handle.arguments.first().copied();
    let spec_path_str = spec_path_expr
        .and_then(|id| try_get_plain_text_literal(module, id))
        .map(|s| s.to_owned());

    if let Some(ref spec_path) = spec_path_str {
        let path = std::path::Path::new(spec_path.as_str());
        match aivi_openapi::parse_spec_and_find_operation(path, &operation_id) {
            Some(info) => {
                if !info.method.is_read_only() {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "operation `{operation_id}` is a {} mutation; use `value` instead of `signal` for write operations",
                            info.method.as_str()
                        ))
                        .with_code(code("api-mutation-used-as-signal"))
                        .with_label(DiagnosticLabel::primary(
                            invocation.span,
                            "bind this as a `value` so it is executed as a one-shot command",
                        )),
                    );
                    return None;
                }
                let op_path_expr = synthesize_text_literal(module, &info.path, invocation.span);
                Some(SourceDecorator {
                    provider: Some(provider_name_path(
                        invocation.span,
                        BuiltinSourceProvider::ApiGet,
                    )),
                    arguments: vec![spec_path_expr.unwrap_or(op_path_expr), op_path_expr],
                    options: handle.options,
                })
            }
            None => {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "operation `{operation_id}` not found in OpenAPI spec `{spec_path}`"
                    ))
                    .with_code(code("unknown-api-operation"))
                    .with_label(DiagnosticLabel::primary(
                        invocation.span,
                        "check the operationId in the spec file",
                    )),
                );
                None
            }
        }
    } else {
        let op_id_expr = synthesize_text_literal(module, &operation_id, invocation.span);
        let spec_arg = spec_path_expr.unwrap_or(op_id_expr);
        Some(SourceDecorator {
            provider: Some(provider_name_path(
                invocation.span,
                BuiltinSourceProvider::ApiGet,
            )),
            arguments: vec![spec_arg, op_id_expr],
            options: handle.options,
        })
    }
}

fn lower_api_value_member(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    invocation: &CapabilityInvocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    let operation_id = invocation.member.clone();
    let spec_path_expr = handle.arguments.first().copied();
    let spec_path_str = spec_path_expr
        .and_then(|id| try_get_plain_text_literal(module, id))
        .map(|s| s.to_owned());

    if let Some(ref spec_path) = spec_path_str {
        let path = std::path::Path::new(spec_path.as_str());
        match aivi_openapi::parse_spec_and_find_operation(path, &operation_id) {
            Some(info) => {
                if info.method.is_read_only() {
                    return None;
                }
                let intrinsic = match info.method {
                    aivi_openapi::OperationMethod::Post => IntrinsicValue::HttpPost,
                    aivi_openapi::OperationMethod::Put => IntrinsicValue::HttpPut,
                    aivi_openapi::OperationMethod::Delete => IntrinsicValue::HttpDelete,
                    aivi_openapi::OperationMethod::Patch => IntrinsicValue::HttpPut,
                    _ => return None,
                };
                let arguments = combine_http_value_arguments(
                    module,
                    handle,
                    &invocation.arguments,
                    invocation.span,
                    diagnostics,
                );
                Some(build_intrinsic_call(
                    module,
                    intrinsic,
                    invocation.span,
                    arguments,
                ))
            }
            None => {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "operation `{operation_id}` not found in OpenAPI spec `{spec_path}`"
                    ))
                    .with_code(code("unknown-api-operation"))
                    .with_label(DiagnosticLabel::primary(
                        invocation.span,
                        "check the operationId in the spec file",
                    )),
                );
                None
            }
        }
    } else {
        let arguments = combine_http_value_arguments(
            module,
            handle,
            &invocation.arguments,
            invocation.span,
            diagnostics,
        );
        Some(build_intrinsic_call(
            module,
            IntrinsicValue::HttpPost,
            invocation.span,
            arguments,
        ))
    }
}

fn combine_http_value_arguments(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    member_arguments: &[ExprId],
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ExprId> {
    if member_arguments.is_empty() {
        return handle.arguments.clone();
    }
    let mut arguments = member_arguments.to_vec();
    if let Some(url) =
        scoped_http_url_argument(module, handle, Some(member_arguments[0]), span, diagnostics)
    {
        arguments[0] = url;
    }
    arguments
}

fn scoped_path_argument(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    member_argument: Option<ExprId>,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    match (handle.arguments.as_slice(), member_argument) {
        ([], Some(argument)) => Some(argument),
        ([], None) => None,
        ([base], Some(argument)) => Some(build_intrinsic_call(
            module,
            IntrinsicValue::PathJoin,
            span,
            vec![*base, argument],
        )),
        ([base], None) => Some(*base),
        ([first, ..], Some(argument)) => {
            diagnostics.push(
                Diagnostic::error(
                    "file-system capability handles currently accept at most one inherited root path",
                )
                .with_code(code("invalid-source-capability-arguments"))
                .with_label(DiagnosticLabel::primary(
                    handle.span,
                    "collapse the base path to one expression before declaring the handle",
                )),
            );
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::PathJoin,
                span,
                vec![*first, argument],
            ))
        }
        ([first, ..], None) => {
            diagnostics.push(
                Diagnostic::error(
                    "file-system capability handles currently accept at most one inherited root path",
                )
                .with_code(code("invalid-source-capability-arguments"))
                .with_label(DiagnosticLabel::primary(
                    handle.span,
                    "collapse the base path to one expression before declaring the handle",
                )),
            );
            Some(*first)
        }
    }
}

fn scoped_http_url_argument(
    module: &mut Module,
    handle: &CapabilityHandleBinding,
    member_argument: Option<ExprId>,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ExprId> {
    match (handle.arguments.as_slice(), member_argument) {
        ([], Some(argument)) => Some(argument),
        ([], None) => None,
        ([base], Some(argument)) => Some(build_intrinsic_call(
            module,
            IntrinsicValue::TextConcat,
            span,
            vec![*base, argument],
        )),
        ([base], None) => Some(*base),
        ([first, ..], Some(argument)) => {
            diagnostics.push(
                Diagnostic::error(
                    "HTTP capability handles currently accept at most one inherited base URL expression",
                )
                .with_code(code("invalid-source-capability-arguments"))
                .with_label(DiagnosticLabel::primary(
                    handle.span,
                    "collapse the base URL to one expression before declaring the handle",
                )),
            );
            Some(build_intrinsic_call(
                module,
                IntrinsicValue::TextConcat,
                span,
                vec![*first, argument],
            ))
        }
        ([first, ..], None) => {
            diagnostics.push(
                Diagnostic::error(
                    "HTTP capability handles currently accept at most one inherited base URL expression",
                )
                .with_code(code("invalid-source-capability-arguments"))
                .with_label(DiagnosticLabel::primary(
                    handle.span,
                    "collapse the base URL to one expression before declaring the handle",
                )),
            );
            Some(*first)
        }
    }
}

fn build_intrinsic_call(
    module: &mut Module,
    intrinsic: IntrinsicValue,
    span: SourceSpan,
    arguments: Vec<ExprId>,
) -> ExprId {
    let callee = module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                intrinsic_name_path(intrinsic, span),
                TermResolution::IntrinsicValue(intrinsic),
            )),
        })
        .expect("capability lowering should fit inside the expression arena");
    if arguments.is_empty() {
        return callee;
    }
    let arguments = NonEmpty::from_vec(arguments)
        .expect("capability intrinsic applications always pass at least one argument");
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Apply { callee, arguments },
        })
        .expect("capability lowering should fit inside the expression arena")
}

fn synthesize_source_decorator(span: SourceSpan, payload: SourceDecorator) -> Decorator {
    Decorator {
        span,
        name: name_path(span, &["source"]),
        payload: DecoratorPayload::Source(payload),
    }
}

fn apply_signal_rewrite(module: &mut Module, rewrite: SignalCapabilityRewrite) {
    let decorator_id = module
        .alloc_decorator(rewrite.rewrite.decorator)
        .expect("capability lowering should fit inside the decorator arena");
    let Some(Item::Signal(signal)) = module.arenas.items.get_mut(rewrite.item_id) else {
        return;
    };
    signal.body = None;
    signal.header.decorators.push(decorator_id);
}

fn apply_value_rewrite(module: &mut Module, rewrite: ValueCapabilityRewrite) {
    let Some(Item::Value(value)) = module.arenas.items.get_mut(rewrite.item_id) else {
        return;
    };
    value.body = rewrite.body;
}

fn signal_with_source_decorator(
    module: &Module,
    item_id: ItemId,
) -> Option<(&SignalItem, &SourceDecorator)> {
    let Item::Signal(signal) = &module.items()[item_id] else {
        return None;
    };
    let source = signal.header.decorators.iter().find_map(|decorator_id| {
        let decorator = &module.decorators()[*decorator_id];
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some(source),
            _ => None,
        }
    })?;
    Some((signal, source))
}

fn signal_has_source_decorator(module: &Module, signal: &SignalItem) -> bool {
    signal.header.decorators.iter().any(|decorator_id| {
        matches!(
            module
                .decorators()
                .get(*decorator_id)
                .map(|decorator| &decorator.payload),
            Some(DecoratorPayload::Source(_))
        )
    })
}

fn classify_handle_provider(path: &NamePath) -> HandleProviderClassification {
    if let Some(family) = builtin_capability_family(path) {
        return HandleProviderClassification::BuiltinFamily(family);
    }
    match SourceProviderRef::from_path(Some(path)) {
        SourceProviderRef::Builtin(provider) => {
            HandleProviderClassification::BuiltinProvider(provider)
        }
        SourceProviderRef::Custom(key) => HandleProviderClassification::Custom(key),
        SourceProviderRef::InvalidShape(_) => HandleProviderClassification::InvalidShape,
        SourceProviderRef::Missing => {
            unreachable!("explicit provider paths should never classify as missing")
        }
    }
}

fn builtin_capability_family(path: &NamePath) -> Option<BuiltinCapabilityFamily> {
    if path.segments().len() != 1 {
        return None;
    }
    match path.segments().first().text() {
        "fs" => Some(BuiltinCapabilityFamily::Fs),
        "http" => Some(BuiltinCapabilityFamily::Http),
        "db" => Some(BuiltinCapabilityFamily::Db),
        "env" => Some(BuiltinCapabilityFamily::Env),
        "log" => Some(BuiltinCapabilityFamily::Log),
        "stdio" => Some(BuiltinCapabilityFamily::Stdio),
        "random" => Some(BuiltinCapabilityFamily::Random),
        "process" => Some(BuiltinCapabilityFamily::Process),
        "path" => Some(BuiltinCapabilityFamily::Path),
        "dbus" => Some(BuiltinCapabilityFamily::Dbus),
        "imap" => Some(BuiltinCapabilityFamily::Imap),
        "smtp" => Some(BuiltinCapabilityFamily::Smtp),
        "time" => Some(BuiltinCapabilityFamily::Time),
        "api" => Some(BuiltinCapabilityFamily::Api),
        _ => None,
    }
}

fn supports_builtin_signal_member(family: BuiltinCapabilityFamily, member: &str) -> bool {
    match family {
        BuiltinCapabilityFamily::Fs => matches!(member, "read" | "watch"),
        BuiltinCapabilityFamily::Http => matches!(member, "get"),
        BuiltinCapabilityFamily::Db => matches!(member, "connect" | "live"),
        BuiltinCapabilityFamily::Env => matches!(member, "get"),
        BuiltinCapabilityFamily::Stdio => matches!(member, "read"),
        BuiltinCapabilityFamily::Process => matches!(member, "spawn" | "args" | "cwd"),
        BuiltinCapabilityFamily::Path => matches!(
            member,
            "home" | "configHome" | "dataHome" | "cacheHome" | "tempDir"
        ),
        BuiltinCapabilityFamily::Dbus => matches!(member, "ownName" | "signal" | "method"),
        BuiltinCapabilityFamily::Imap => matches!(member, "connect" | "idle" | "fetchBody"),
        BuiltinCapabilityFamily::Time => matches!(member, "nowMs"),
        BuiltinCapabilityFamily::Log
        | BuiltinCapabilityFamily::Random
        | BuiltinCapabilityFamily::Smtp => false,
        // Api members are dynamic (spec-based); validation happens in lower_api_signal_member.
        BuiltinCapabilityFamily::Api => true,
    }
}

fn supports_builtin_value_member(family: BuiltinCapabilityFamily, member: &str) -> bool {
    match family {
        BuiltinCapabilityFamily::Fs => matches!(
            member,
            "read"
                | "readText"
                | "readBytes"
                | "readDir"
                | "exists"
                | "write"
                | "writeText"
                | "writeBytes"
                | "createDirAll"
                | "delete"
                | "deleteFile"
                | "deleteDir"
                | "rename"
                | "move"
                | "copy"
        ),
        BuiltinCapabilityFamily::Http => matches!(
            member,
            "get" | "getBytes" | "getStatus" | "post" | "put" | "delete" | "head" | "postJson"
        ),
        BuiltinCapabilityFamily::Db => matches!(member, "query" | "commit" | "exec"),
        BuiltinCapabilityFamily::Env => matches!(member, "get" | "list"),
        BuiltinCapabilityFamily::Log => matches!(member, "emit" | "emitContext"),
        BuiltinCapabilityFamily::Stdio => {
            matches!(
                member,
                "write" | "stdoutWrite" | "writeError" | "stderrWrite"
            )
        }
        BuiltinCapabilityFamily::Random => matches!(
            member,
            "randomInt" | "int" | "randomBytes" | "bytes" | "randomFloat" | "float"
        ),
        BuiltinCapabilityFamily::Path => matches!(
            member,
            "dataHome"
                | "configHome"
                | "cacheHome"
                | "stateHome"
                | "runtimeDir"
                | "dataDirs"
                | "configDirs"
        ),
        BuiltinCapabilityFamily::Process
        | BuiltinCapabilityFamily::Dbus
        | BuiltinCapabilityFamily::Imap
        | BuiltinCapabilityFamily::Time => false,
        BuiltinCapabilityFamily::Smtp => matches!(member, "send"),
        // Api members are dynamic (spec-based); validation happens in lower_api_value_member.
        BuiltinCapabilityFamily::Api => true,
    }
}

fn intrinsic_name_path(intrinsic: IntrinsicValue, span: SourceSpan) -> NamePath {
    let rendered = intrinsic.to_string();
    let segments = rendered.split('.').collect::<Vec<_>>();
    name_path(span, &segments)
}

fn provider_name_path(span: SourceSpan, provider: BuiltinSourceProvider) -> NamePath {
    let segments = provider.key().split('.').collect::<Vec<_>>();
    name_path(span, &segments)
}

fn provider_key_name_path(span: SourceSpan, key: &str) -> NamePath {
    let segments = key.split('.').collect::<Vec<_>>();
    name_path(span, &segments)
}

fn name_path(span: SourceSpan, segments: &[&str]) -> NamePath {
    NamePath::from_vec(
        segments
            .iter()
            .map(|segment| {
                Name::new(*segment, span).expect("compiler-generated names should be valid")
            })
            .collect(),
    )
    .expect("compiler-generated name paths should be valid")
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

#[derive(Clone, Debug)]
struct SignalCapabilityRewrite {
    item_id: ItemId,
    rewrite: SignalRewritePlan,
}

#[derive(Clone, Debug)]
struct SignalRewritePlan {
    decorator: Decorator,
}

#[derive(Clone, Debug)]
struct ValueCapabilityRewrite {
    item_id: ItemId,
    body: ExprId,
}

#[derive(Clone, Debug)]
enum HandleProviderClassification {
    BuiltinFamily(BuiltinCapabilityFamily),
    BuiltinProvider(BuiltinSourceProvider),
    Custom(Box<str>),
    InvalidShape,
}
