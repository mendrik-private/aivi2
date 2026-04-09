impl<'a> RuntimeFragmentLowerer<'a> {
    fn new(hir: &'a aivi_hir::Module, fragment: &'a RuntimeFragmentSpec) -> Self {
        let report = elaborate_general_expressions(hir);
        let completeness_errors = validate_general_expr_report_completeness(hir, &report, |_| true);
        let (items, domain_members, instance_members) = report.into_parts();
        let mut report_by_owner: HashMap<HirItemId, _> =
            items.into_iter().map(|item| (item.owner, item)).collect();
        // Ambient prelude items are elaborated separately; merge their results so
        // runtime fragments that reference ambient functions can lower them.
        let ambient_report = elaborate_ambient_items(hir);
        let (ambient_items, ambient_domain_members, _) = ambient_report.into_parts();
        for item in ambient_items {
            report_by_owner.entry(item.owner).or_insert(item);
        }
        let mut domain_member_reports: HashMap<_, _> = domain_members
            .into_iter()
            .map(|item| {
                (
                    DomainMemberKey {
                        domain: item.domain_owner,
                        member_index: item.member_index,
                    },
                    item,
                )
            })
            .collect();
        for member in ambient_domain_members {
            domain_member_reports
                .entry(DomainMemberKey {
                    domain: member.domain_owner,
                    member_index: member.member_index,
                })
                .or_insert(member);
        }
        let instance_member_reports = instance_members
            .into_iter()
            .map(|item| {
                (
                    InstanceMemberKey {
                        instance: item.instance_owner,
                        member_index: item.member_index,
                    },
                    item,
                )
            })
            .collect();
        let mut lowerer = Self {
            lowerer: ModuleLowerer::new(hir),
            fragment,
            report_by_owner,
            domain_member_reports,
            instance_member_reports,
            lowering: HashSet::new(),
            lowered: HashSet::new(),
            lowering_domain_members: HashSet::new(),
            lowered_domain_members: HashSet::new(),
            lowering_instance_members: HashSet::new(),
            lowered_instance_members: HashSet::new(),
        };
        lowerer.lowerer.errors.extend(completeness_errors);
        lowerer
    }

    fn build(mut self) -> Result<LoweredRuntimeFragment, LoweringErrors> {
        // Guard: reject incomplete elaboration before walking dependencies.
        let has_completeness_errors = self.lowerer.errors.iter().any(|e| {
            matches!(
                e,
                LoweringError::MissingGeneralExprElaboration { .. }
                    | LoweringError::MissingDomainMemberElaboration { .. }
                    | LoweringError::MissingInstanceMemberElaboration { .. }
            )
        });
        if has_completeness_errors {
            return Err(LoweringErrors::new(self.lowerer.errors));
        }
        let dependencies = referenced_hir_dependencies(&self.fragment.body);
        for dependency in dependencies.items {
            self.ensure_hir_item_lowered(dependency);
        }
        for dependency in dependencies.domain_members {
            self.ensure_domain_member_lowered(dependency);
        }
        for dependency in dependencies.instance_members {
            self.ensure_instance_member_lowered(dependency);
        }

        let fragment_item = self
            .lowerer
            .module
            .items_mut()
            .alloc(Item {
                origin: self.fragment.owner,
                span: self.lowerer.hir.exprs()[self.fragment.body_expr].span,
                name: self.fragment.name.clone(),
                kind: if self.fragment.parameters.is_empty() {
                    ItemKind::Value
                } else {
                    ItemKind::Function
                },
                parameters: self
                    .fragment
                    .parameters
                    .iter()
                    .map(|parameter| ItemParameter {
                        binding: parameter.binding,
                        span: parameter.span,
                        name: parameter.name.clone(),
                        ty: Type::lower(&parameter.ty),
                    })
                    .collect(),
                body: None,
                pipes: Vec::new(),
            })
            .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("items", overflow)]))?;

        match self
            .lowerer
            .lower_runtime_expr(self.fragment.owner, &self.fragment.body)
        {
            Ok(body) => {
                let item = self
                    .lowerer
                    .module
                    .items_mut()
                    .get_mut(fragment_item)
                    .expect("freshly allocated runtime fragment item should exist");
                item.body = Some(body);
            }
            Err(error) => self.lowerer.errors.push(error),
        }

        if !self.lowerer.errors.is_empty() {
            return Err(LoweringErrors::new(self.lowerer.errors));
        }
        if let Err(validation) = validate_module(&self.lowerer.module) {
            self.lowerer.errors.extend(
                validation
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::Validation),
            );
            return Err(LoweringErrors::new(self.lowerer.errors));
        }

        Ok(LoweredRuntimeFragment {
            entry_name: self.fragment.name.clone(),
            module: self.lowerer.module,
        })
    }

    fn ensure_hir_item_lowered(&mut self, owner: HirItemId) {
        if self.lowered.contains(&owner) || self.lowering.contains(&owner) {
            return;
        }
        match self.lowerer.hir.items().get(owner) {
            Some(HirItem::Signal(_)) => {
                if self.seed_hir_item(owner).is_some() {
                    self.lowered.insert(owner);
                }
                return;
            }
            // Domain/Type/Class/Use/Export/SourceProviderContract items do not produce
            // core-level items — only their members do (handled via
            // ensure_domain_member_lowered / ensure_instance_member_lowered). Silently
            // skip them so transitive dependency walks don't trigger UnknownOwner.
            Some(
                HirItem::Domain(_)
                | HirItem::Type(_)
                | HirItem::Class(_)
                | HirItem::SourceProviderContract(_)
                | HirItem::Use(_)
                | HirItem::Export(_),
            )
            | None => return,
            _ => {}
        }
        let Some(report) = self.report_by_owner.get(&owner).cloned() else {
            let is_ambient = self.lowerer.hir.ambient_items().contains(&owner);
            if is_ambient {
                // Ambient items are elaborated separately; just seed them without a body.
                self.seed_hir_item(owner);
                return;
            }
            self.lowerer
                .errors
                .push(LoweringError::UnknownOwner { owner });
            return;
        };
        let Some(core_item) = self.seed_hir_item(owner) else {
            return;
        };
        let body = match report.outcome {
            GeneralExprOutcome::Lowered(body) => body,
            GeneralExprOutcome::Blocked(blocked) => {
                self.lowerer.errors.push(LoweringError::BlockedGeneralExpr {
                    owner,
                    body_expr: report.body_expr,
                    span: blocked.primary_span().unwrap_or_default(),
                    blocked,
                });
                return;
            }
        };

        self.lowering.insert(owner);
        let dependencies = referenced_hir_dependencies(&body);
        for dependency in dependencies.items {
            self.ensure_hir_item_lowered(dependency);
        }
        for dependency in dependencies.domain_members {
            self.ensure_domain_member_lowered(dependency);
        }
        for dependency in dependencies.instance_members {
            self.ensure_instance_member_lowered(dependency);
        }
        if self.lowerer.errors.is_empty() {
            match self.lowerer.lower_runtime_expr(owner, &body) {
                Ok(lowered_body) => {
                    let item = self
                        .lowerer
                        .module
                        .items_mut()
                        .get_mut(core_item)
                        .expect("seeded runtime dependency item should exist");
                    item.parameters = report
                        .parameters
                        .iter()
                        .map(|parameter| ItemParameter {
                            binding: parameter.binding,
                            span: parameter.span,
                            name: parameter.name.clone(),
                            ty: Type::lower(&parameter.ty),
                        })
                        .collect();
                    item.body = Some(lowered_body);
                }
                Err(error) => self.lowerer.errors.push(error),
            }
        }
        self.lowering.remove(&owner);
        self.lowered.insert(owner);
    }

    fn ensure_domain_member_lowered(&mut self, key: DomainMemberKey) {
        if self.lowered_domain_members.contains(&key) || self.lowering_domain_members.contains(&key)
        {
            return;
        }
        let Some(report) = self.domain_member_reports.get(&key).cloned() else {
            return;
        };
        let Some(core_item) = self
            .lowerer
            .seed_domain_member_item(key.domain, key.member_index)
        else {
            return;
        };
        let body = match report.outcome {
            GeneralExprOutcome::Lowered(body) => body,
            GeneralExprOutcome::Blocked(blocked) => {
                self.lowerer.errors.push(LoweringError::BlockedGeneralExpr {
                    owner: key.domain,
                    body_expr: report.body_expr,
                    span: blocked.primary_span().unwrap_or_default(),
                    blocked,
                });
                return;
            }
        };

        self.lowering_domain_members.insert(key);
        let dependencies = referenced_hir_dependencies(&body);
        for dependency in dependencies.items {
            self.ensure_hir_item_lowered(dependency);
        }
        for dependency in dependencies.domain_members {
            self.ensure_domain_member_lowered(dependency);
        }
        for dependency in dependencies.instance_members {
            self.ensure_instance_member_lowered(dependency);
        }
        if self.lowerer.errors.is_empty() {
            match self.lowerer.lower_runtime_expr(key.domain, &body) {
                Ok(lowered_body) => {
                    let item = self
                        .lowerer
                        .module
                        .items_mut()
                        .get_mut(core_item)
                        .expect("seeded runtime dependency item should exist");
                    item.parameters = report
                        .parameters
                        .iter()
                        .map(|parameter| ItemParameter {
                            binding: parameter.binding,
                            span: parameter.span,
                            name: parameter.name.clone(),
                            ty: Type::lower(&parameter.ty),
                        })
                        .collect();
                    item.body = Some(lowered_body);
                }
                Err(error) => self.lowerer.errors.push(error),
            }
        }
        self.lowering_domain_members.remove(&key);
        self.lowered_domain_members.insert(key);
    }

    fn ensure_instance_member_lowered(&mut self, key: InstanceMemberKey) {
        if self.lowered_instance_members.contains(&key)
            || self.lowering_instance_members.contains(&key)
        {
            return;
        }
        let Some(report) = self.instance_member_reports.get(&key).cloned() else {
            self.lowerer.errors.push(LoweringError::UnknownOwner {
                owner: key.instance,
            });
            return;
        };
        let Some(core_item) = self
            .lowerer
            .seed_instance_member_item(key.instance, key.member_index)
        else {
            return;
        };
        let body = match report.outcome {
            GeneralExprOutcome::Lowered(body) => body,
            GeneralExprOutcome::Blocked(blocked) => {
                self.lowerer.errors.push(LoweringError::BlockedGeneralExpr {
                    owner: key.instance,
                    body_expr: report.body_expr,
                    span: blocked.primary_span().unwrap_or_default(),
                    blocked,
                });
                return;
            }
        };

        self.lowering_instance_members.insert(key);
        let dependencies = referenced_hir_dependencies(&body);
        for dependency in dependencies.items {
            self.ensure_hir_item_lowered(dependency);
        }
        for dependency in dependencies.domain_members {
            self.ensure_domain_member_lowered(dependency);
        }
        for dependency in dependencies.instance_members {
            self.ensure_instance_member_lowered(dependency);
        }
        if self.lowerer.errors.is_empty() {
            match self.lowerer.lower_runtime_expr(key.instance, &body) {
                Ok(lowered_body) => {
                    let item = self
                        .lowerer
                        .module
                        .items_mut()
                        .get_mut(core_item)
                        .expect("seeded runtime dependency item should exist");
                    item.parameters = report
                        .parameters
                        .iter()
                        .map(|parameter| ItemParameter {
                            binding: parameter.binding,
                            span: parameter.span,
                            name: parameter.name.clone(),
                            ty: Type::lower(&parameter.ty),
                        })
                        .collect();
                    item.body = Some(lowered_body);
                }
                Err(error) => self.lowerer.errors.push(error),
            }
        }
        self.lowering_instance_members.remove(&key);
        self.lowered_instance_members.insert(key);
    }

    fn seed_hir_item(&mut self, owner: HirItemId) -> Option<ItemId> {
        if let Some(item) = self.lowerer.item_map.get(&owner).copied() {
            return Some(item);
        }
        let item = self.lowerer.hir.items().get(owner)?;
        let (span, name, kind) = match item {
            HirItem::Value(item) => (item.header.span, item.name.text().into(), ItemKind::Value),
            HirItem::Function(item) => (
                item.header.span,
                item.name.text().into(),
                ItemKind::Function,
            ),
            HirItem::Signal(item) => (
                item.header.span,
                item.name.text().into(),
                ItemKind::Signal(SignalInfo::default()),
            ),
            HirItem::Instance(item) => (
                item.header.span,
                format!("instance#{}", owner.as_raw()).into_boxed_str(),
                ItemKind::Instance,
            ),
            // These item types do not produce core-level items.
            HirItem::Type(_)
            | HirItem::Class(_)
            | HirItem::Domain(_)
            | HirItem::SourceProviderContract(_)
            | HirItem::Use(_)
            | HirItem::Export(_)
            | HirItem::Hoist(_) => return None,
        };
        let item_id = match self.lowerer.module.items_mut().alloc(Item {
            origin: owner,
            span,
            name,
            kind,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        }) {
            Ok(item_id) => item_id,
            Err(overflow) => {
                self.lowerer.errors.push(arena_overflow("items", overflow));
                return None;
            }
        };
        self.lowerer.item_map.insert(owner, item_id);
        Some(item_id)
    }
}

impl<'a> RuntimeFragmentItemCollector<'a> {
    fn new(hir: &'a aivi_hir::Module, fragment: &'a RuntimeFragmentSpec) -> Self {
        let (items, domain_members, instance_members) =
            elaborate_general_expressions(hir).into_parts();
        let mut report_by_owner: HashMap<HirItemId, _> =
            items.into_iter().map(|item| (item.owner, item)).collect();
        // Include ambient prelude items so fragment dependency collection can
        // transitively walk through ambient function bodies.
        let (ambient_items, ambient_domain_members, _) = elaborate_ambient_items(hir).into_parts();
        for item in ambient_items {
            report_by_owner.entry(item.owner).or_insert(item);
        }
        let mut domain_member_reports: HashMap<_, _> = domain_members
            .into_iter()
            .map(|item| {
                (
                    DomainMemberKey {
                        domain: item.domain_owner,
                        member_index: item.member_index,
                    },
                    item,
                )
            })
            .collect();
        for member in ambient_domain_members {
            domain_member_reports
                .entry(DomainMemberKey {
                    domain: member.domain_owner,
                    member_index: member.member_index,
                })
                .or_insert(member);
        }
        let instance_member_reports = instance_members
            .into_iter()
            .map(|item| {
                (
                    InstanceMemberKey {
                        instance: item.instance_owner,
                        member_index: item.member_index,
                    },
                    item,
                )
            })
            .collect();
        Self {
            hir,
            fragment,
            report_by_owner,
            domain_member_reports,
            instance_member_reports,
            included_items: HashSet::new(),
            visited_domain_members: HashSet::new(),
            visited_instance_members: HashSet::new(),
        }
    }

    fn collect(mut self) -> HashSet<HirItemId> {
        self.collect_item(self.fragment.owner);
        self.collect_mock_replacements(self.fragment.owner);
        let dependencies = referenced_hir_dependencies(&self.fragment.body);
        for dependency in dependencies.items {
            self.collect_item(dependency);
        }
        for dependency in dependencies.domain_members {
            self.collect_domain_member(dependency);
        }
        for dependency in dependencies.instance_members {
            self.collect_instance_member(dependency);
        }
        self.included_items
    }

    fn collect_item(&mut self, owner: HirItemId) {
        if !self.included_items.insert(owner) {
            return;
        }
        let Some(item) = self.hir.items().get(owner) else {
            return;
        };
        self.collect_mock_replacements(owner);
        match item {
            HirItem::Signal(signal) => {
                for dependency in &signal.signal_dependencies {
                    self.collect_item(*dependency);
                }
                if let Some(source_metadata) = &signal.source_metadata {
                    for dependency in &source_metadata.signal_dependencies {
                        self.collect_item(*dependency);
                    }
                    for dependency in source_metadata.lifecycle_dependencies.merged() {
                        self.collect_item(dependency);
                    }
                }
            }
            HirItem::Instance(instance) => {
                for member_index in 0..instance.members.len() {
                    self.collect_instance_member(InstanceMemberKey {
                        instance: owner,
                        member_index,
                    });
                }
            }
            HirItem::Value(_) | HirItem::Function(_) => {
                let Some(report) = self.report_by_owner.get(&owner) else {
                    return;
                };
                let GeneralExprOutcome::Lowered(body) = &report.outcome else {
                    return;
                };
                let dependencies = referenced_hir_dependencies(body);
                for dependency in dependencies.items {
                    self.collect_item(dependency);
                }
                for dependency in dependencies.domain_members {
                    self.collect_domain_member(dependency);
                }
                for dependency in dependencies.instance_members {
                    self.collect_instance_member(dependency);
                }
            }
            HirItem::Type(_)
            | HirItem::Class(_)
            | HirItem::SourceProviderContract(_)
            | HirItem::Use(_)
            | HirItem::Export(_)
            | HirItem::Hoist(_) => {}
            HirItem::Domain(domain) => {
                for (member_index, member) in domain.members.iter().enumerate() {
                    if member.body.is_none() {
                        continue;
                    }
                    self.collect_domain_member(DomainMemberKey {
                        domain: owner,
                        member_index,
                    });
                }
            }
        }
    }

    fn collect_domain_member(&mut self, key: DomainMemberKey) {
        if !self.visited_domain_members.insert(key) {
            return;
        }
        self.collect_item(key.domain);
        let Some(report) = self.domain_member_reports.get(&key) else {
            return;
        };
        let GeneralExprOutcome::Lowered(body) = &report.outcome else {
            return;
        };
        let dependencies = referenced_hir_dependencies(body);
        for dependency in dependencies.items {
            self.collect_item(dependency);
        }
        for dependency in dependencies.domain_members {
            self.collect_domain_member(dependency);
        }
        for dependency in dependencies.instance_members {
            self.collect_instance_member(dependency);
        }
    }

    fn collect_instance_member(&mut self, key: InstanceMemberKey) {
        if !self.visited_instance_members.insert(key) {
            return;
        }
        self.collect_item(key.instance);
        let Some(report) = self.instance_member_reports.get(&key) else {
            return;
        };
        let GeneralExprOutcome::Lowered(body) = &report.outcome else {
            return;
        };
        let dependencies = referenced_hir_dependencies(body);
        for dependency in dependencies.items {
            self.collect_item(dependency);
        }
        for dependency in dependencies.domain_members {
            self.collect_domain_member(dependency);
        }
        for dependency in dependencies.instance_members {
            self.collect_instance_member(dependency);
        }
    }

    fn collect_mock_replacements(&mut self, owner: HirItemId) {
        let Some(item) = self.hir.items().get(owner) else {
            return;
        };
        for decorator_id in item.decorators() {
            let Some(decorator) = self.hir.decorators().get(*decorator_id) else {
                continue;
            };
            let DecoratorPayload::Mock(mock) = &decorator.payload else {
                continue;
            };
            let Some(MockImportTarget::Item(item_id)) =
                mock_replacement_target(self.hir, mock.replacement)
            else {
                continue;
            };
            self.collect_item(item_id);
        }
    }
}

#[derive(Default)]
struct HirDependencies {
    items: Vec<HirItemId>,
    domain_members: Vec<DomainMemberKey>,
    instance_members: Vec<InstanceMemberKey>,
}

fn referenced_hir_dependencies(root: &GateRuntimeExpr) -> HirDependencies {
    let mut seen_items = HashSet::new();
    let mut seen_domain_members = HashSet::new();
    let mut seen_instance_members = HashSet::new();
    let mut work = vec![root];
    while let Some(expr) = work.pop() {
        match &expr.kind {
            GateRuntimeExprKind::AmbientSubject
            | GateRuntimeExprKind::Integer(_)
            | GateRuntimeExprKind::Float(_)
            | GateRuntimeExprKind::Decimal(_)
            | GateRuntimeExprKind::BigInt(_)
            | GateRuntimeExprKind::SuffixedInteger(_)
            | GateRuntimeExprKind::Reference(GateRuntimeReference::Local(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::Builtin(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::IntrinsicValue(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::Import(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::SumConstructor(_)) => {}
            GateRuntimeExprKind::Reference(GateRuntimeReference::Item(item)) => {
                seen_items.insert(*item);
            }
            GateRuntimeExprKind::Reference(GateRuntimeReference::DomainMember(handle)) => {
                seen_domain_members.insert(DomainMemberKey {
                    domain: handle.domain,
                    member_index: handle.member_index,
                });
            }
            GateRuntimeExprKind::Reference(GateRuntimeReference::ClassMember(dispatch)) => {
                if let aivi_hir::ClassMemberImplementation::SameModuleInstance {
                    instance,
                    member_index,
                } = dispatch.implementation
                {
                    seen_instance_members.insert(InstanceMemberKey {
                        instance,
                        member_index,
                    });
                }
            }
            GateRuntimeExprKind::Text(text) => {
                for segment in text.segments.iter().rev() {
                    if let GateRuntimeTextSegment::Interpolation(interpolation) = segment {
                        work.push(interpolation);
                    }
                }
            }
            GateRuntimeExprKind::Tuple(elements)
            | GateRuntimeExprKind::List(elements)
            | GateRuntimeExprKind::Set(elements) => {
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            GateRuntimeExprKind::Map(entries) => {
                for entry in entries.iter().rev() {
                    work.push(&entry.value);
                    work.push(&entry.key);
                }
            }
            GateRuntimeExprKind::Record(fields) => {
                for field in fields.iter().rev() {
                    work.push(&field.value);
                }
            }
            GateRuntimeExprKind::Projection { base, .. } => {
                if let GateRuntimeProjectionBase::Expr(base) = base {
                    work.push(base);
                }
            }
            GateRuntimeExprKind::Apply { callee, arguments } => {
                for argument in arguments.iter().rev() {
                    work.push(argument);
                }
                work.push(callee);
            }
            GateRuntimeExprKind::Unary { expr, .. } => work.push(expr),
            GateRuntimeExprKind::Binary { left, right, .. } => {
                work.push(right);
                work.push(left);
            }
            GateRuntimeExprKind::Pipe(pipe) => {
                work.push(&pipe.head);
                for stage in pipe.stages.iter().rev() {
                    match &stage.kind {
                        GateRuntimePipeStageKind::Transform { expr, .. }
                        | GateRuntimePipeStageKind::Tap { expr }
                        | GateRuntimePipeStageKind::Gate {
                            predicate: expr, ..
                        }
                        | GateRuntimePipeStageKind::FanOut { map_expr: expr } => work.push(expr),
                        GateRuntimePipeStageKind::Case { arms } => {
                            for arm in arms.iter().rev() {
                                work.push(&arm.body);
                            }
                        }
                        GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                            work.push(&falsy.body);
                            work.push(&truthy.body);
                        }
                    }
                }
            }
        }
    }
    let mut items = seen_items.into_iter().collect::<Vec<_>>();
    items.sort_by_key(|item| item.as_raw());
    let mut domain_members = seen_domain_members.into_iter().collect::<Vec<_>>();
    domain_members.sort();
    let mut instance_members = seen_instance_members.into_iter().collect::<Vec<_>>();
    instance_members.sort();
    HirDependencies {
        items,
        domain_members,
        instance_members,
    }
}

