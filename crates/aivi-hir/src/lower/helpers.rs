
/// Build a string label from an `ImportValueType`, used for generating unique synthetic
/// binding names for auto-imported instance members.
fn import_value_type_label(ty: &ImportValueType) -> String {
    match ty {
        ImportValueType::Primitive(builtin) => format!("{builtin:?}"),
        ImportValueType::Tuple(elements) => {
            let parts: Vec<_> = elements.iter().map(import_value_type_label).collect();
            format!("Tuple_{}", parts.join("_"))
        }
        ImportValueType::Record(fields) => {
            let parts: Vec<_> = fields
                .iter()
                .map(|f| format!("{}_{}", f.name, import_value_type_label(&f.ty)))
                .collect();
            format!("Record_{}", parts.join("_"))
        }
        ImportValueType::Arrow { parameter, result } => {
            format!(
                "Arrow_{}_{}",
                import_value_type_label(parameter),
                import_value_type_label(result)
            )
        }
        ImportValueType::List(elem) => format!("List_{}", import_value_type_label(elem)),
        ImportValueType::Map { key, value } => {
            format!(
                "Map_{}_{}",
                import_value_type_label(key),
                import_value_type_label(value)
            )
        }
        ImportValueType::Set(elem) => format!("Set_{}", import_value_type_label(elem)),
        ImportValueType::Option(elem) => format!("Option_{}", import_value_type_label(elem)),
        ImportValueType::Result { error, value } => {
            format!(
                "Result_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::Validation { error, value } => {
            format!(
                "Validation_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::Signal(elem) => format!("Signal_{}", import_value_type_label(elem)),
        ImportValueType::Task { error, value } => {
            format!(
                "Task_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::TypeVariable { name, .. } => name.clone(),
        ImportValueType::Named {
            type_name,
            arguments,
            ..
        } => {
            if arguments.is_empty() {
                type_name.clone()
            } else {
                let args: Vec<_> = arguments.iter().map(import_value_type_label).collect();
                format!("{}_{}", type_name, args.join("_"))
            }
        }
    }
}

impl ResolveEnv {
    fn push_term_scope(&mut self, scope: HashMap<String, BindingId>) {
        self.term_scopes.push(scope);
    }

    fn push_type_scope(&mut self, scope: HashMap<String, TypeParameterId>) {
        self.type_scopes.push(scope);
    }

    fn enable_implicit_type_parameters(&mut self) {
        self.allow_implicit_type_parameters = true;
        if self.type_scopes.is_empty() {
            self.type_scopes.push(HashMap::new());
        }
    }

    fn set_prefer_ambient_names(&mut self) {
        self.prefer_ambient_names = true;
    }

    fn lookup_term(&self, name: &str) -> Option<BindingId> {
        self.term_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn lookup_type(&self, name: &str) -> Option<TypeParameterId> {
        self.type_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn allow_implicit_type_parameters(&self) -> bool {
        self.allow_implicit_type_parameters
    }

    fn prefer_ambient_names(&self) -> bool {
        self.prefer_ambient_names
    }

    fn bind_implicit_type_parameter(
        &mut self,
        name: &str,
        span: SourceSpan,
        lowerer: &mut Lowerer,
    ) -> TypeParameterId {
        if let Some(parameter) = self.lookup_type(name) {
            return parameter;
        }
        if self.type_scopes.is_empty() {
            self.type_scopes.push(HashMap::new());
        }
        let parameter = lowerer.alloc_type_parameter(TypeParameter {
            span,
            name: lowerer.make_name(name, span),
        });
        self.type_scopes
            .last_mut()
            .expect("implicit type parameter scope should exist")
            .insert(name.to_owned(), parameter);
        self.implicit_type_parameters.push(parameter);
        parameter
    }

    fn implicit_type_parameters(&self) -> Vec<TypeParameterId> {
        self.implicit_type_parameters.clone()
    }
}

fn is_implicit_type_parameter_candidate(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
}

impl LoweredMarkup {
    fn span(&self, lowerer: &Lowerer) -> SourceSpan {
        match self {
            Self::Renderable(id) => lowerer.module.markup_nodes()[*id].span,
            Self::Empty(id) | Self::Case(id) => lowerer.module.control_nodes()[*id].span(),
        }
    }
}

fn lower_unary_operator(operator: syn::UnaryOperator) -> UnaryOperator {
    match operator {
        syn::UnaryOperator::Not => UnaryOperator::Not,
    }
}

fn lower_binary_operator(operator: syn::BinaryOperator) -> BinaryOperator {
    match operator {
        syn::BinaryOperator::Add => BinaryOperator::Add,
        syn::BinaryOperator::Subtract => BinaryOperator::Subtract,
        syn::BinaryOperator::Multiply => BinaryOperator::Multiply,
        syn::BinaryOperator::Divide => BinaryOperator::Divide,
        syn::BinaryOperator::Modulo => BinaryOperator::Modulo,
        syn::BinaryOperator::GreaterThan => BinaryOperator::GreaterThan,
        syn::BinaryOperator::LessThan => BinaryOperator::LessThan,
        syn::BinaryOperator::GreaterThanOrEqual => BinaryOperator::GreaterThanOrEqual,
        syn::BinaryOperator::LessThanOrEqual => BinaryOperator::LessThanOrEqual,
        syn::BinaryOperator::Equals => BinaryOperator::Equals,
        syn::BinaryOperator::NotEquals => BinaryOperator::NotEquals,
        syn::BinaryOperator::And => BinaryOperator::And,
        syn::BinaryOperator::Or => BinaryOperator::Or,
    }
}

fn insert_named(
    map: &mut HashMap<String, Vec<NamedSite<ItemId>>>,
    name: &str,
    item_id: ItemId,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
    error_code: DiagnosticCode,
    subject: &str,
) {
    let entry = map.entry(name.to_owned()).or_default();
    if let Some(previous) = entry.first().copied() {
        diagnostics.push(
            Diagnostic::error(format!("duplicate {subject} name `{name}`"))
                .with_code(error_code)
                .with_primary_label(span, "this declaration reuses an existing top-level name")
                .with_secondary_label(previous.span, "previous declaration here"),
        );
    }
    entry.push(NamedSite {
        value: item_id,
        span,
    });
}

fn insert_site<T: Copy>(
    map: &mut HashMap<String, Vec<NamedSite<T>>>,
    name: &str,
    value: T,
    span: SourceSpan,
) {
    map.entry(name.to_owned())
        .or_default()
        .push(NamedSite { value, span });
}

fn lookup_item<T: Copy>(map: &HashMap<String, Vec<NamedSite<T>>>, name: &str) -> LookupResult<T> {
    match map.get(name) {
        Some(values) if values.len() == 1 => LookupResult::Unique(values[0].value),
        Some(_) => LookupResult::Ambiguous,
        None => LookupResult::Missing,
    }
}

fn is_source_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "source"
}

fn is_test_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "test"
}

fn is_debug_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "debug"
}

fn is_deprecated_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "deprecated"
}

fn is_mock_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "mock"
}

fn recurrence_wakeup_decorator_kind(path: &NamePath) -> Option<RecurrenceWakeupDecoratorKind> {
    match path_text(path).as_str() {
        "recur.timer" => Some(RecurrenceWakeupDecoratorKind::Timer),
        "recur.backoff" => Some(RecurrenceWakeupDecoratorKind::Backoff),
        _ => None,
    }
}

fn is_recurrence_wakeup_decorator_name(path: &syn::QualifiedName) -> bool {
    matches!(path.as_dotted().as_str(), "recur.timer" | "recur.backoff")
}

fn path_text(path: &NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}

fn domain_member_surface_name(name: &syn::DomainMemberName) -> String {
    match name {
        syn::DomainMemberName::Signature(syn::ClassMemberName::Identifier(identifier)) => {
            identifier.text.clone()
        }
        syn::DomainMemberName::Signature(syn::ClassMemberName::Operator(operator)) => {
            format!("({})", operator.text)
        }
        syn::DomainMemberName::Literal(identifier) => format!("literal {}", identifier.text),
    }
}

fn domain_member_surface_key(name: &syn::DomainMemberName) -> String {
    match name {
        syn::DomainMemberName::Signature(syn::ClassMemberName::Identifier(identifier)) => {
            format!("method:{}", identifier.text)
        }
        syn::DomainMemberName::Signature(syn::ClassMemberName::Operator(operator)) => {
            format!("operator:{}", operator.text)
        }
        syn::DomainMemberName::Literal(identifier) => format!("literal:{}", identifier.text),
    }
}

fn builtin_term(name: &str) -> Option<BuiltinTerm> {
    match name {
        "True" => Some(BuiltinTerm::True),
        "False" => Some(BuiltinTerm::False),
        "None" => Some(BuiltinTerm::None),
        "Some" => Some(BuiltinTerm::Some),
        "Ok" => Some(BuiltinTerm::Ok),
        "Err" => Some(BuiltinTerm::Err),
        "Valid" => Some(BuiltinTerm::Valid),
        "Invalid" => Some(BuiltinTerm::Invalid),
        _ => None,
    }
}

fn builtin_type(name: &str) -> Option<BuiltinType> {
    match name {
        "Int" => Some(BuiltinType::Int),
        "Float" => Some(BuiltinType::Float),
        "Decimal" => Some(BuiltinType::Decimal),
        "BigInt" => Some(BuiltinType::BigInt),
        "Bool" => Some(BuiltinType::Bool),
        "Text" => Some(BuiltinType::Text),
        "Unit" => Some(BuiltinType::Unit),
        "Bytes" => Some(BuiltinType::Bytes),
        "List" => Some(BuiltinType::List),
        "Map" => Some(BuiltinType::Map),
        "Set" => Some(BuiltinType::Set),
        "Option" => Some(BuiltinType::Option),
        "Result" => Some(BuiltinType::Result),
        "Validation" => Some(BuiltinType::Validation),
        "Signal" => Some(BuiltinType::Signal),
        "Task" => Some(BuiltinType::Task),
        _ => None,
    }
}

fn is_known_module(module: &str) -> bool {
    matches!(
        module,
        "aivi.network"
            | "aivi.defaults"
            | "aivi.random"
            | "aivi.stdio"
            | "aivi.fs"
            | "aivi.db"
            | "aivi.text"
            | "aivi.time"
            | "aivi.env"
            | "aivi.i18n"
            | "aivi.log"
            | "aivi.regex"
            | "aivi.http"
            | "aivi.bigint"
            | "aivi.nonEmpty"
            | "aivi.matrix"
            | "aivi.option"
            | "aivi.list"
    )
}

fn known_import_metadata(module: &str, member: &str) -> Option<ImportBindingMetadata> {
    match (module, member) {
        ("aivi.network", "http") | ("aivi.network", "socket") => {
            Some(ImportBindingMetadata::Value {
                ty: ImportValueType::Primitive(BuiltinType::Text),
            })
        }
        ("aivi.network", "Request") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(1),
            fields: None,
            definition: None,
        }),
        ("aivi.network", "Channel") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(2),
            fields: None,
            definition: None,
        }),
        ("aivi.defaults", "defaultText") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Text),
        }),
        ("aivi.defaults", "defaultInt") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Int),
        }),
        ("aivi.defaults", "defaultBool") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Bool),
        }),
        ("aivi.defaults", "Option") => Some(ImportBindingMetadata::Bundle(
            ImportBundleKind::BuiltinOption,
        )),
        ("aivi.option", "getOrElse") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_option_getOrElse".into(),
        }),
        ("aivi.result", "withDefault") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_result_withDefault".into(),
        }),
        ("aivi.list", "length") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_length".into(),
        }),
        ("aivi.list", "head") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_head".into(),
        }),
        ("aivi.list", "tail") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_tail".into(),
        }),
        ("aivi.list", "tailOrEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_tailOrEmpty".into(),
        }),
        ("aivi.list", "nonEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_nonEmpty".into(),
        }),
        ("aivi.list", "any") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_any".into(),
        }),
        ("aivi.list", "at") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_listAt".into(),
        }),
        ("aivi.list", "replaceAt") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_listReplace".into(),
        }),
        ("aivi.stdio", "stdoutWrite") => Some(intrinsic_import_value(
            IntrinsicValue::StdoutWrite,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.stdio", "stderrWrite") => Some(intrinsic_import_value(
            IntrinsicValue::StderrWrite,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "createDirAll") => Some(intrinsic_import_value(
            IntrinsicValue::FsCreateDirAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "deleteFile") => Some(intrinsic_import_value(
            IntrinsicValue::FsDeleteFile,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "writeText") => Some(intrinsic_import_value(
            IntrinsicValue::FsWriteText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Unit),
                    ),
                ),
            ),
        )),
        ("aivi.fs", "exists") => Some(intrinsic_import_value(
            IntrinsicValue::FsExists,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.random", "RandomError") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(0),
            fields: None,
            definition: None,
        }),
        ("aivi.db", "paramBool") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bool),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramInt") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramFloat") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamFloat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramDecimal") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamDecimal,
            arrow_import_type(
                primitive_import_type(BuiltinType::Decimal),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramBigInt") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBigInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramText") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramBytes") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBytes,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "statement") => Some(intrinsic_import_value(
            IntrinsicValue::DbStatement,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    list_import_type(db_param_import_type()),
                    db_statement_import_type(),
                ),
            ),
        )),
        // Float math intrinsics
        ("aivi.core.float", "floor") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFloor,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "ceil") => Some(intrinsic_import_value(
            IntrinsicValue::FloatCeil,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "round") => Some(intrinsic_import_value(
            IntrinsicValue::FloatRound,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "sqrt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatSqrt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "abs") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAbs,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "toInt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatToInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.core.float", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::FloatToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.core.float", "parseText") => Some(intrinsic_import_value(
            IntrinsicValue::FloatParseText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        // Path intrinsics — synchronous, operate on Text path strings
        ("aivi.path", "parent") => Some(intrinsic_import_value(
            IntrinsicValue::PathParent,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "filename") => Some(intrinsic_import_value(
            IntrinsicValue::PathFilename,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "stem") => Some(intrinsic_import_value(
            IntrinsicValue::PathStem,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "extension") => Some(intrinsic_import_value(
            IntrinsicValue::PathExtension,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "join") => Some(intrinsic_import_value(
            IntrinsicValue::PathJoin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.path", "isAbsolute") => Some(intrinsic_import_value(
            IntrinsicValue::PathIsAbsolute,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Bool),
            ),
        )),
        ("aivi.path", "normalize") => Some(intrinsic_import_value(
            IntrinsicValue::PathNormalize,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        // Bytes intrinsics — synchronous operations on the Bytes type
        ("aivi.core.bytes", "length") => Some(intrinsic_import_value(
            IntrinsicValue::BytesLength,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.core.bytes", "get") => Some(intrinsic_import_value(
            IntrinsicValue::BytesGet,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Bytes),
                    option_import_type(primitive_import_type(BuiltinType::Int)),
                ),
            ),
        )),
        ("aivi.core.bytes", "slice") => Some(intrinsic_import_value(
            IntrinsicValue::BytesSlice,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Bytes),
                        primitive_import_type(BuiltinType::Bytes),
                    ),
                ),
            ),
        )),
        ("aivi.core.bytes", "append") => Some(intrinsic_import_value(
            IntrinsicValue::BytesAppend,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Bytes),
                    primitive_import_type(BuiltinType::Bytes),
                ),
            ),
        )),
        ("aivi.core.bytes", "fromText") => Some(intrinsic_import_value(
            IntrinsicValue::BytesFromText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Bytes),
            ),
        )),
        ("aivi.core.bytes", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::BytesToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.core.bytes", "repeat") => Some(intrinsic_import_value(
            IntrinsicValue::BytesRepeat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    primitive_import_type(BuiltinType::Bytes),
                ),
            ),
        )),
        ("aivi.core.bytes", "empty") => Some(intrinsic_import_value(
            IntrinsicValue::BytesEmpty,
            primitive_import_type(BuiltinType::Bytes),
        )),
        // JSON intrinsics — async tasks, executed via serde_json in CLI
        ("aivi.data.json", "validate") => Some(intrinsic_import_value(
            IntrinsicValue::JsonValidate,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.data.json", "get") => Some(intrinsic_import_value(
            IntrinsicValue::JsonGet,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.data.json", "at") => Some(intrinsic_import_value(
            IntrinsicValue::JsonAt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.data.json", "keys") => Some(intrinsic_import_value(
            IntrinsicValue::JsonKeys,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    list_import_type(primitive_import_type(BuiltinType::Text)),
                ),
            ),
        )),
        ("aivi.data.json", "pretty") => Some(intrinsic_import_value(
            IntrinsicValue::JsonPretty,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.data.json", "minify") => Some(intrinsic_import_value(
            IntrinsicValue::JsonMinify,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        // XDG base directory intrinsics — synchronous, no I/O cost beyond env-var reads
        ("aivi.desktop.xdg", "dataHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgDataHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "configHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgConfigHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "cacheHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgCacheHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "stateHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgStateHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "runtimeDir") => Some(intrinsic_import_value(
            IntrinsicValue::XdgRuntimeDir,
            option_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        ("aivi.desktop.xdg", "dataDirs") => Some(intrinsic_import_value(
            IntrinsicValue::XdgDataDirs,
            list_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        ("aivi.desktop.xdg", "configDirs") => Some(intrinsic_import_value(
            IntrinsicValue::XdgConfigDirs,
            list_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        // Text intrinsics
        ("aivi.text", "length") => Some(intrinsic_import_value(
            IntrinsicValue::TextLength,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.text", "byteLen") => Some(intrinsic_import_value(
            IntrinsicValue::TextByteLen,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.text", "slice") => Some(intrinsic_import_value(
            IntrinsicValue::TextSlice,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "find") => Some(intrinsic_import_value(
            IntrinsicValue::TextFind,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    option_import_type(primitive_import_type(BuiltinType::Int)),
                ),
            ),
        )),
        ("aivi.text", "contains") => Some(intrinsic_import_value(
            IntrinsicValue::TextContains,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "startsWith") => Some(intrinsic_import_value(
            IntrinsicValue::TextStartsWith,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "endsWith") => Some(intrinsic_import_value(
            IntrinsicValue::TextEndsWith,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "toUpper") => Some(intrinsic_import_value(
            IntrinsicValue::TextToUpper,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "toLower") => Some(intrinsic_import_value(
            IntrinsicValue::TextToLower,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trim") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrim,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trimStart") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrimStart,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trimEnd") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrimEnd,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "replace") => Some(intrinsic_import_value(
            IntrinsicValue::TextReplace,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "replaceAll") => Some(intrinsic_import_value(
            IntrinsicValue::TextReplaceAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "split") => Some(intrinsic_import_value(
            IntrinsicValue::TextSplit,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    list_import_type(primitive_import_type(BuiltinType::Text)),
                ),
            ),
        )),
        ("aivi.text", "repeat") => Some(intrinsic_import_value(
            IntrinsicValue::TextRepeat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.text", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::TextFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "parseInt") => Some(intrinsic_import_value(
            IntrinsicValue::TextParseInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Int)),
            ),
        )),
        ("aivi.text", "fromBool") => Some(intrinsic_import_value(
            IntrinsicValue::TextFromBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bool),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "parseBool") => Some(intrinsic_import_value(
            IntrinsicValue::TextParseBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Bool)),
            ),
        )),
        ("aivi.text", "concat") => Some(intrinsic_import_value(
            IntrinsicValue::TextConcat,
            arrow_import_type(
                list_import_type(primitive_import_type(BuiltinType::Text)),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        // Float transcendental intrinsics
        ("aivi.core.float", "sin") => Some(intrinsic_import_value(
            IntrinsicValue::FloatSin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "cos") => Some(intrinsic_import_value(
            IntrinsicValue::FloatCos,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "tan") => Some(intrinsic_import_value(
            IntrinsicValue::FloatTan,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "asin") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAsin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "acos") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAcos,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "atan") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAtan,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "atan2") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAtan2,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    primitive_import_type(BuiltinType::Float),
                ),
            ),
        )),
        ("aivi.core.float", "exp") => Some(intrinsic_import_value(
            IntrinsicValue::FloatExp,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "log") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "log2") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog2,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "log10") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog10,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "pow") => Some(intrinsic_import_value(
            IntrinsicValue::FloatPow,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    option_import_type(primitive_import_type(BuiltinType::Float)),
                ),
            ),
        )),
        ("aivi.core.float", "hypot") => Some(intrinsic_import_value(
            IntrinsicValue::FloatHypot,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    primitive_import_type(BuiltinType::Float),
                ),
            ),
        )),
        ("aivi.core.float", "trunc") => Some(intrinsic_import_value(
            IntrinsicValue::FloatTrunc,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "frac") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFrac,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        // Time intrinsics
        ("aivi.time", "nowMs") => Some(intrinsic_import_value(
            IntrinsicValue::TimeNowMs,
            task_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.time", "monotonicMs") => Some(intrinsic_import_value(
            IntrinsicValue::TimeMonotonicMs,
            task_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.time", "format") => Some(intrinsic_import_value(
            IntrinsicValue::TimeFormat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.time", "parse") => Some(intrinsic_import_value(
            IntrinsicValue::TimeParse,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Int),
                    ),
                ),
            ),
        )),
        // Regex intrinsics
        ("aivi.regex", "isMatch") => Some(intrinsic_import_value(
            IntrinsicValue::RegexIsMatch,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Bool),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "find") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFind,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Int)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "findText") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFindText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "findAll") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFindAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        list_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "replace") => Some(intrinsic_import_value(
            IntrinsicValue::RegexReplace,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        task_import_type(
                            primitive_import_type(BuiltinType::Text),
                            primitive_import_type(BuiltinType::Text),
                        ),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "replaceAll") => Some(intrinsic_import_value(
            IntrinsicValue::RegexReplaceAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        task_import_type(
                            primitive_import_type(BuiltinType::Text),
                            primitive_import_type(BuiltinType::Text),
                        ),
                    ),
                ),
            ),
        )),
        // I18n intrinsics
        ("aivi.i18n", "tr") => Some(intrinsic_import_value(
            IntrinsicValue::I18nTranslate,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.i18n", "trn") => Some(intrinsic_import_value(
            IntrinsicValue::I18nTranslatePlural,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Int),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        // BigInt intrinsics (pure/synchronous)
        ("aivi.bigint", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "fromText") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntFromText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::BigInt)),
            ),
        )),
        ("aivi.bigint", "toInt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntToInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                option_import_type(primitive_import_type(BuiltinType::Int)),
            ),
        )),
        ("aivi.bigint", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.bigint", "add") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntAdd,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "sub") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntSub,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "mul") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntMul,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "div") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntDiv,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    option_import_type(primitive_import_type(BuiltinType::BigInt)),
                ),
            ),
        )),
        ("aivi.bigint", "mod") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntMod,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    option_import_type(primitive_import_type(BuiltinType::BigInt)),
                ),
            ),
        )),
        ("aivi.bigint", "pow") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntPow,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "neg") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntNeg,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "abs") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntAbs,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "cmp") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntCmp,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Int),
                ),
            ),
        )),
        ("aivi.bigint", "eq") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntEq,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.bigint", "gt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntGt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.bigint", "lt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntLt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        // NonEmptyList ambient types and values
        ("aivi.nonEmpty", "NonEmptyList") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.nonEmpty", "singleton") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_singleton".into(),
        }),
        ("aivi.nonEmpty", "head") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_head".into(),
        }),
        ("aivi.nonEmpty", "cons") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_cons".into(),
        }),
        ("aivi.nonEmpty", "length") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_length".into(),
        }),
        ("aivi.nonEmpty", "toList") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_toList".into(),
        }),
        ("aivi.nonEmpty", "init") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_init".into(),
        }),
        ("aivi.nonEmpty", "fromHeadTail") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_fromHeadTail".into(),
        }),
        ("aivi.nonEmpty", "last") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_last".into(),
        }),
        ("aivi.nonEmpty", "mapNel") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_mapNel".into(),
        }),
        ("aivi.nonEmpty", "appendNel") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_appendNel".into(),
        }),
        ("aivi.nonEmpty", "fromList") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_fromList".into(),
        }),
        // Option ambient values
        ("aivi.option", "map") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_option_map".into(),
        }),
        // List ambient values
        ("aivi.list", "contains") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_contains".into(),
        }),
        ("aivi.list", "map") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_map".into(),
        }),
        ("aivi.list", "flatMap") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_flatMap".into(),
        }),
        ("aivi.list", "filter") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_filter".into(),
        }),
        ("aivi.list", "count") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_count".into(),
        }),
        ("aivi.list", "sum") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_sum".into(),
        }),
        ("aivi.list", "maximum") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_maximum".into(),
        }),
        // Text ambient values
        ("aivi.text", "join") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_join".into(),
        }),
        ("aivi.text", "lower") => Some(intrinsic_import_value(
            IntrinsicValue::TextToLower,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "isEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_isEmpty".into(),
        }),
        ("aivi.text", "nonEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_nonEmpty".into(),
        }),
        ("aivi.list", "find") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_find".into(),
        }),
        ("aivi.list", "take") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_take".into(),
        }),
        ("aivi.list", "sortBy") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_sortBy".into(),
        }),
        // Matrix ambient types and values
        ("aivi.matrix", "Matrix") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.matrix", "MatrixError") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.matrix", "indices") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_range".into(),
        }),
        ("aivi.matrix", "fromRows") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_fromRows".into(),
        }),
        ("aivi.matrix", "at") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_at".into(),
        }),
        ("aivi.matrix", "replaceAt") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_replaceAt".into(),
        }),
        ("aivi.matrix", "rows") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_rows".into(),
        }),
        ("aivi.matrix", "width") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_width".into(),
        }),
        ("aivi.matrix", "height") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_height".into(),
        }),
        ("aivi.matrix", "count") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_count".into(),
        }),
        ("aivi.matrix", "filled") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_filled".into(),
        }),
        ("aivi.matrix", "init") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_init".into(),
        }),
        _ => None,
    }
}

fn intrinsic_import_value(value: IntrinsicValue, ty: ImportValueType) -> ImportBindingMetadata {
    ImportBindingMetadata::IntrinsicValue { value, ty }
}

fn primitive_import_type(builtin: BuiltinType) -> ImportValueType {
    ImportValueType::Primitive(builtin)
}

fn record_import_type(fields: Vec<ImportRecordField>) -> ImportValueType {
    ImportValueType::Record(fields)
}

fn record_import_field(name: &str, ty: ImportValueType) -> ImportRecordField {
    ImportRecordField {
        name: name.into(),
        ty,
    }
}

fn arrow_import_type(parameter: ImportValueType, result: ImportValueType) -> ImportValueType {
    ImportValueType::Arrow {
        parameter: Box::new(parameter),
        result: Box::new(result),
    }
}

fn task_import_type(error: ImportValueType, value: ImportValueType) -> ImportValueType {
    ImportValueType::Task {
        error: Box::new(error),
        value: Box::new(value),
    }
}

fn option_import_type(element: ImportValueType) -> ImportValueType {
    ImportValueType::Option(Box::new(element))
}

fn list_import_type(element: ImportValueType) -> ImportValueType {
    ImportValueType::List(Box::new(element))
}

fn db_param_import_type() -> ImportValueType {
    record_import_type(vec![
        record_import_field("kind", primitive_import_type(BuiltinType::Text)),
        record_import_field(
            "bool",
            option_import_type(primitive_import_type(BuiltinType::Bool)),
        ),
        record_import_field(
            "int",
            option_import_type(primitive_import_type(BuiltinType::Int)),
        ),
        record_import_field(
            "float",
            option_import_type(primitive_import_type(BuiltinType::Float)),
        ),
        record_import_field(
            "decimal",
            option_import_type(primitive_import_type(BuiltinType::Decimal)),
        ),
        record_import_field(
            "bigInt",
            option_import_type(primitive_import_type(BuiltinType::BigInt)),
        ),
        record_import_field(
            "text",
            option_import_type(primitive_import_type(BuiltinType::Text)),
        ),
        record_import_field(
            "bytes",
            option_import_type(primitive_import_type(BuiltinType::Bytes)),
        ),
    ])
}

fn db_statement_import_type() -> ImportValueType {
    record_import_type(vec![
        record_import_field("sql", primitive_import_type(BuiltinType::Text)),
        record_import_field("arguments", list_import_type(db_param_import_type())),
    ])
}

fn surface_exprs_equal(left: &syn::Expr, right: &syn::Expr) -> bool {
    match (&left.kind, &right.kind) {
        (syn::ExprKind::Group(left), _) => surface_exprs_equal(left, right),
        (_, syn::ExprKind::Group(right)) => surface_exprs_equal(left, right),
        (syn::ExprKind::Name(left), syn::ExprKind::Name(right)) => left.text == right.text,
        (syn::ExprKind::Integer(left), syn::ExprKind::Integer(right)) => left.raw == right.raw,
        (syn::ExprKind::Float(left), syn::ExprKind::Float(right)) => left.raw == right.raw,
        (syn::ExprKind::Decimal(left), syn::ExprKind::Decimal(right)) => left.raw == right.raw,
        (syn::ExprKind::BigInt(left), syn::ExprKind::BigInt(right)) => left.raw == right.raw,
        (syn::ExprKind::SuffixedInteger(left), syn::ExprKind::SuffixedInteger(right)) => {
            left.literal.raw == right.literal.raw && left.suffix.text == right.suffix.text
        }
        (syn::ExprKind::Text(left), syn::ExprKind::Text(right)) => {
            left.segments.len() == right.segments.len()
                && left
                    .segments
                    .iter()
                    .zip(&right.segments)
                    .all(|(left, right)| match (left, right) {
                        (syn::TextSegment::Text(left), syn::TextSegment::Text(right)) => {
                            left.raw == right.raw
                        }
                        (
                            syn::TextSegment::Interpolation(left),
                            syn::TextSegment::Interpolation(right),
                        ) => surface_exprs_equal(&left.expr, &right.expr),
                        _ => false,
                    })
        }
        (syn::ExprKind::Regex(left), syn::ExprKind::Regex(right)) => left.raw == right.raw,
        (syn::ExprKind::Tuple(left), syn::ExprKind::Tuple(right))
        | (syn::ExprKind::List(left), syn::ExprKind::List(right))
        | (syn::ExprKind::Set(left), syn::ExprKind::Set(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| surface_exprs_equal(left, right))
        }
        (syn::ExprKind::Map(left), syn::ExprKind::Map(right)) => {
            left.entries.len() == right.entries.len()
                && left
                    .entries
                    .iter()
                    .zip(&right.entries)
                    .all(|(left, right)| {
                        surface_exprs_equal(&left.key, &right.key)
                            && surface_exprs_equal(&left.value, &right.value)
                    })
        }
        (syn::ExprKind::Record(left), syn::ExprKind::Record(right)) => {
            left.fields.len() == right.fields.len()
                && left.fields.iter().zip(&right.fields).all(|(left, right)| {
                    left.label.text == right.label.text
                        && match (&left.value, &right.value) {
                            (Some(left), Some(right)) => surface_exprs_equal(left, right),
                            (None, None) => true,
                            (None, Some(value)) | (Some(value), None) => {
                                matches!(
                                    &value.kind,
                                    syn::ExprKind::Name(identifier)
                                        if identifier.text == left.label.text
                                )
                            }
                        }
                })
        }
        (syn::ExprKind::SubjectPlaceholder, syn::ExprKind::SubjectPlaceholder) => true,
        (syn::ExprKind::AmbientProjection(left), syn::ExprKind::AmbientProjection(right)) => {
            left.fields.len() == right.fields.len()
                && left
                    .fields
                    .iter()
                    .zip(&right.fields)
                    .all(|(left, right)| left.text == right.text)
        }
        (
            syn::ExprKind::Range {
                start: left_start,
                end: left_end,
            },
            syn::ExprKind::Range {
                start: right_start,
                end: right_end,
            },
        ) => {
            surface_exprs_equal(left_start, right_start) && surface_exprs_equal(left_end, right_end)
        }
        (
            syn::ExprKind::Projection {
                base: left_base,
                path: left_path,
            },
            syn::ExprKind::Projection {
                base: right_base,
                path: right_path,
            },
        ) => {
            surface_exprs_equal(left_base, right_base)
                && left_path.fields.len() == right_path.fields.len()
                && left_path
                    .fields
                    .iter()
                    .zip(&right_path.fields)
                    .all(|(left, right)| left.text == right.text)
        }
        (
            syn::ExprKind::Apply {
                callee: left_callee,
                arguments: left_arguments,
            },
            syn::ExprKind::Apply {
                callee: right_callee,
                arguments: right_arguments,
            },
        ) => {
            surface_exprs_equal(left_callee, right_callee)
                && left_arguments.len() == right_arguments.len()
                && left_arguments
                    .iter()
                    .zip(right_arguments)
                    .all(|(left, right)| surface_exprs_equal(left, right))
        }
        (
            syn::ExprKind::Unary {
                operator: left_operator,
                expr: left_expr,
            },
            syn::ExprKind::Unary {
                operator: right_operator,
                expr: right_expr,
            },
        ) => left_operator == right_operator && surface_exprs_equal(left_expr, right_expr),
        (
            syn::ExprKind::Binary {
                left: left_left,
                operator: left_operator,
                right: left_right,
            },
            syn::ExprKind::Binary {
                left: right_left,
                operator: right_operator,
                right: right_right,
            },
        ) => {
            left_operator == right_operator
                && surface_exprs_equal(left_left, right_left)
                && surface_exprs_equal(left_right, right_right)
        }
        (syn::ExprKind::ResultBlock(left), syn::ExprKind::ResultBlock(right)) => {
            left.bindings.len() == right.bindings.len()
                && left
                    .bindings
                    .iter()
                    .zip(&right.bindings)
                    .all(|(left, right)| {
                        left.name.text == right.name.text
                            && surface_exprs_equal(&left.expr, &right.expr)
                    })
                && match (&left.tail, &right.tail) {
                    (Some(left), Some(right)) => surface_exprs_equal(left, right),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

/// Iterative Levenshtein distance between two strings (character-level).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j].min(dp[i][j - 1]).min(dp[i - 1][j - 1])
            };
        }
    }
    dp[m][n]
}

/// Return the closest candidate within Levenshtein distance 2 of `target`, or `None`.
fn closest_name<'a>(target: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .filter_map(|&c| {
            let d = levenshtein(target, c);
            if d <= 2 { Some((d, c)) } else { None }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name)
}

