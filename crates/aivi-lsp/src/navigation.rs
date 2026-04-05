use std::sync::Arc;

use aivi_base::{ByteIndex, LspPosition, SourceSpan, Span};
use aivi_hir::{
    BinaryOperator, BindingId, BuiltinTerm, BuiltinType, ClassMemberResolution, DomainMemberKind,
    DomainMemberResolution, ExportResolution, ExprKind, ImportBinding, ImportBindingMetadata,
    ImportBindingResolution, ImportId, Item, ItemId, LiteralSuffixResolution, Module, NamePath,
    PatternKind, ResolutionState, TermResolution, TypeItemBody, TypeKind, TypeParameterId,
    TypeResolution,
};
use aivi_query::{HirModuleResult, RootDatabase, SourceFile};
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationTarget {
    file: SourceFile,
    pub(crate) span: SourceSpan,
}

impl NavigationTarget {
    fn new(file: SourceFile, span: SourceSpan) -> Self {
        Self { file, span }
    }

    /// Try to find the `LspSymbol` declared at this target's span.  Used by
    /// hover to retrieve the declaration's type detail when the cursor is on a
    /// reference site rather than the declaration itself.
    pub fn find_symbol_at_target(&self, db: &RootDatabase) -> Option<aivi_hir::LspSymbol> {
        let hir = aivi_query::hir_module(db, self.file);
        let symbols = hir.symbols_arc();
        let target_span = self.span;
        let mut stack: Vec<aivi_hir::LspSymbol> = symbols.iter().cloned().collect();
        let mut best: Option<aivi_hir::LspSymbol> = None;
        while let Some(sym) = stack.pop() {
            if sym.span.file() == target_span.file()
                && sym.span.span().contains(target_span.span().start())
            {
                if best.as_ref().is_none_or(|b: &aivi_hir::LspSymbol| {
                    sym.span.span().len() < b.span.span().len()
                }) {
                    best = Some(sym.clone());
                }
                stack.extend(sym.children.iter().cloned());
            }
        }
        best
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationLookup {
    NoSite,
    NoTargets,
    Targets(Vec<NavigationTarget>),
}

impl NavigationLookup {
    fn from_targets(targets: Vec<NavigationTarget>) -> Self {
        if targets.is_empty() {
            Self::NoTargets
        } else {
            Self::Targets(targets)
        }
    }
}

pub struct NavigationAnalysis {
    file: SourceFile,
    hir: Arc<HirModuleResult>,
    source: Arc<aivi_base::SourceFile>,
}

impl NavigationAnalysis {
    pub fn load(db: &RootDatabase, file: SourceFile) -> Self {
        let hir = aivi_query::hir_module(db, file);
        let source = hir.source_arc();
        Self { file, hir, source }
    }

    pub fn definition_targets_at_lsp_position(
        &self,
        db: &RootDatabase,
        position: LspPosition,
    ) -> NavigationLookup {
        let Some(cursor) = self.source.lsp_position_to_offset(position) else {
            return NavigationLookup::NoSite;
        };
        let Some(site) = self.semantic_site_at_offset(cursor) else {
            return NavigationLookup::NoSite;
        };
        NavigationLookup::from_targets(self.definition_targets_for_site(db, &site))
    }

    pub fn implementation_targets_at_lsp_position(
        &self,
        db: &RootDatabase,
        position: LspPosition,
    ) -> NavigationLookup {
        let Some(cursor) = self.source.lsp_position_to_offset(position) else {
            return NavigationLookup::NoSite;
        };
        let Some(site) = self.semantic_site_at_offset(cursor) else {
            return NavigationLookup::NoSite;
        };
        NavigationLookup::from_targets(self.implementation_targets_for_site(db, &site))
    }

    /// Return all `Location`s in this file that refer to any of the given
    /// definition `targets`.  This powers find-all-references and rename.
    pub fn all_reference_locations_for_targets(
        &self,
        db: &RootDatabase,
        targets: &[NavigationTarget],
    ) -> Vec<Location> {
        let mut locations = Vec::new();
        for (span, site) in self.collect_all_sites() {
            let site_targets = self.definition_targets_for_site(db, &site);
            if site_targets.iter().any(|t| targets.contains(t)) {
                if let Some(loc) =
                    location_for_target(db, NavigationTarget::new(self.file, span))
                {
                    if !locations.contains(&loc) {
                        locations.push(loc);
                    }
                }
            }
        }
        locations
    }

    /// Collect every navigable (span, site) pair in this module without
    /// cursor filtering.  Mirrors the traversal in `semantic_site_at_offset`
    /// but accumulates all entries instead of selecting the tightest one.
    fn collect_all_sites(&self) -> Vec<(SourceSpan, NavigationSite)> {
        let module = self.module();
        let source_id = self.source.id();
        let mut sites: Vec<(SourceSpan, NavigationSite)> = Vec::new();

        macro_rules! push {
            ($span:expr, $site:expr) => {
                if $span.file() == source_id {
                    sites.push(($span, $site));
                }
            };
        }

        for (binding_id, binding) in module.bindings().iter() {
            push!(binding.name.span(), NavigationSite::BindingDecl { binding: binding_id });
        }

        for (parameter_id, parameter) in module.type_parameters().iter() {
            push!(
                parameter.name.span(),
                NavigationSite::TypeParameterDecl { parameter: parameter_id }
            );
        }

        for item_id in module.root_items().iter().copied() {
            let item = &module.items()[item_id];
            match item {
                Item::Type(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            push!(
                                variant.name.span(),
                                NavigationSite::ConstructorDecl {
                                    item: item_id,
                                    variant_name: variant.name.text().into(),
                                }
                            );
                        }
                    }
                }
                Item::Value(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                }
                Item::Function(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                }
                Item::Signal(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                }
                Item::Class(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                    for (member_index, member) in item.members.iter().enumerate() {
                        push!(
                            member.name.span(),
                            NavigationSite::ClassMemberDecl {
                                resolution: ClassMemberResolution {
                                    class: item_id,
                                    member_index,
                                },
                            }
                        );
                    }
                }
                Item::Domain(item) => {
                    push!(item.name.span(), NavigationSite::ItemDecl { item: item_id });
                    for (member_index, member) in item.members.iter().enumerate() {
                        push!(
                            member.name.span(),
                            NavigationSite::DomainMemberDecl {
                                resolution: DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                            }
                        );
                    }
                }
                Item::Instance(item) => {
                    push!(
                        item.class.span(),
                        NavigationSite::TypeReference {
                            name: first_segment_text(&item.class.path),
                            resolution: item.class.resolution.clone(),
                        }
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        push!(
                            member.name.span(),
                            NavigationSite::InstanceMemberDecl {
                                instance: item_id,
                                member_index,
                            }
                        );
                    }
                }
                Item::Use(item) => {
                    for import_id in item.imports.iter().copied() {
                        let import = &module.imports()[import_id];
                        push!(
                            import.imported_name.span(),
                            NavigationSite::ImportName {
                                module: item.module.clone(),
                                import: import_id,
                            }
                        );
                        push!(
                            import.local_name.span(),
                            NavigationSite::ImportName {
                                module: item.module.clone(),
                                import: import_id,
                            }
                        );
                    }
                }
                Item::Export(item) => {
                    push!(
                        item.target.span(),
                        NavigationSite::ExportTarget {
                            name: first_segment_text(&item.target),
                            resolution: item.resolution.clone(),
                        }
                    );
                }
                Item::SourceProviderContract(_) => {}
            }
        }

        for (_, ty) in module.types().iter() {
            if let TypeKind::Name(reference) = &ty.kind {
                push!(
                    reference.span(),
                    NavigationSite::TypeReference {
                        name: first_segment_text(&reference.path),
                        resolution: reference.resolution.clone(),
                    }
                );
            }
        }

        for (_, pattern) in module.patterns().iter() {
            match &pattern.kind {
                PatternKind::Constructor { callee, .. }
                | PatternKind::UnresolvedName(callee) => {
                    push!(
                        callee.span(),
                        NavigationSite::TermReference {
                            name: first_segment_text(&callee.path),
                            resolution: callee.resolution.clone(),
                        }
                    );
                }
                PatternKind::Wildcard
                | PatternKind::Binding(_)
                | PatternKind::Integer(_)
                | PatternKind::Text(_)
                | PatternKind::Tuple(_)
                | PatternKind::List { .. }
                | PatternKind::Record(_) => {}
            }
        }

        for (_, expr) in module.exprs().iter() {
            match &expr.kind {
                ExprKind::Name(reference) => {
                    push!(
                        reference.span(),
                        NavigationSite::TermReference {
                            name: first_segment_text(&reference.path),
                            resolution: reference.resolution.clone(),
                        }
                    );
                }
                ExprKind::SuffixedInteger(literal) => {
                    push!(
                        literal.suffix.span(),
                        NavigationSite::LiteralSuffix {
                            resolution: literal.resolution.clone(),
                        }
                    );
                }
                ExprKind::Binary { left, operator, right } => {
                    if let Some(span) = self.binary_operator_span(*left, *right) {
                        push!(span, NavigationSite::BinaryOperator { operator: *operator });
                    }
                }
                ExprKind::Integer(_)
                | ExprKind::Float(_)
                | ExprKind::Decimal(_)
                | ExprKind::BigInt(_)
                | ExprKind::Text(_)
                | ExprKind::Regex(_)
                | ExprKind::Tuple(_)
                | ExprKind::List(_)
                | ExprKind::Map(_)
                | ExprKind::Set(_)
                | ExprKind::Record(_)
                | ExprKind::AmbientSubject
                | ExprKind::Projection { .. }
                | ExprKind::Apply { .. }
                | ExprKind::Unary { .. }
                | ExprKind::PatchApply { .. }
                | ExprKind::PatchLiteral(_)
                | ExprKind::Pipe(_)
                | ExprKind::Cluster(_)
                | ExprKind::Markup(_) => {}
            }
        }

        sites
    }

    fn module(&self) -> &Module {
        self.hir.module()
    }

    fn semantic_site_at_offset(&self, cursor: ByteIndex) -> Option<NavigationSite> {
        let module = self.module();
        let mut best: Option<(u32, NavigationSite)> = None;

        for (binding_id, binding) in module.bindings().iter() {
            self.consider_site(
                binding.name.span(),
                cursor,
                NavigationSite::BindingDecl {
                    binding: binding_id,
                },
                &mut best,
            );
        }

        for (parameter_id, parameter) in module.type_parameters().iter() {
            self.consider_site(
                parameter.name.span(),
                cursor,
                NavigationSite::TypeParameterDecl {
                    parameter: parameter_id,
                },
                &mut best,
            );
        }

        for item_id in module.root_items().iter().copied() {
            let item = &module.items()[item_id];
            match item {
                Item::Type(item) => {
                    self.consider_site(
                        item.name.span(),
                        cursor,
                        NavigationSite::ItemDecl { item: item_id },
                        &mut best,
                    );
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            self.consider_site(
                                variant.name.span(),
                                cursor,
                                NavigationSite::ConstructorDecl {
                                    item: item_id,
                                    variant_name: variant.name.text().into(),
                                },
                                &mut best,
                            );
                        }
                    }
                }
                Item::Value(item) => self.consider_site(
                    item.name.span(),
                    cursor,
                    NavigationSite::ItemDecl { item: item_id },
                    &mut best,
                ),
                Item::Function(item) => self.consider_site(
                    item.name.span(),
                    cursor,
                    NavigationSite::ItemDecl { item: item_id },
                    &mut best,
                ),
                Item::Signal(item) => self.consider_site(
                    item.name.span(),
                    cursor,
                    NavigationSite::ItemDecl { item: item_id },
                    &mut best,
                ),
                Item::Class(item) => {
                    self.consider_site(
                        item.name.span(),
                        cursor,
                        NavigationSite::ItemDecl { item: item_id },
                        &mut best,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        self.consider_site(
                            member.name.span(),
                            cursor,
                            NavigationSite::ClassMemberDecl {
                                resolution: ClassMemberResolution {
                                    class: item_id,
                                    member_index,
                                },
                            },
                            &mut best,
                        );
                    }
                }
                Item::Domain(item) => {
                    self.consider_site(
                        item.name.span(),
                        cursor,
                        NavigationSite::ItemDecl { item: item_id },
                        &mut best,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        self.consider_site(
                            member.name.span(),
                            cursor,
                            NavigationSite::DomainMemberDecl {
                                resolution: DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                            },
                            &mut best,
                        );
                    }
                }
                Item::Instance(item) => {
                    self.consider_site(
                        item.class.span(),
                        cursor,
                        NavigationSite::TypeReference {
                            name: first_segment_text(&item.class.path),
                            resolution: item.class.resolution.clone(),
                        },
                        &mut best,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        self.consider_site(
                            member.name.span(),
                            cursor,
                            NavigationSite::InstanceMemberDecl {
                                instance: item_id,
                                member_index,
                            },
                            &mut best,
                        );
                    }
                }
                Item::Use(item) => {
                    for import_id in item.imports.iter().copied() {
                        let import = &module.imports()[import_id];
                        self.consider_site(
                            import.imported_name.span(),
                            cursor,
                            NavigationSite::ImportName {
                                module: item.module.clone(),
                                import: import_id,
                            },
                            &mut best,
                        );
                        self.consider_site(
                            import.local_name.span(),
                            cursor,
                            NavigationSite::ImportName {
                                module: item.module.clone(),
                                import: import_id,
                            },
                            &mut best,
                        );
                    }
                }
                Item::Export(item) => self.consider_site(
                    item.target.span(),
                    cursor,
                    NavigationSite::ExportTarget {
                        name: first_segment_text(&item.target),
                        resolution: item.resolution.clone(),
                    },
                    &mut best,
                ),
                Item::SourceProviderContract(_) | Item::Hoist(_) => {}
            }
        }

        for (_, ty) in module.types().iter() {
            if let TypeKind::Name(reference) = &ty.kind {
                self.consider_site(
                    reference.span(),
                    cursor,
                    NavigationSite::TypeReference {
                        name: first_segment_text(&reference.path),
                        resolution: reference.resolution.clone(),
                    },
                    &mut best,
                );
            }
        }

        for (_, pattern) in module.patterns().iter() {
            match &pattern.kind {
                PatternKind::Constructor { callee, .. } | PatternKind::UnresolvedName(callee) => {
                    self.consider_site(
                        callee.span(),
                        cursor,
                        NavigationSite::TermReference {
                            name: first_segment_text(&callee.path),
                            resolution: callee.resolution.clone(),
                        },
                        &mut best,
                    );
                }
                PatternKind::Wildcard
                | PatternKind::Binding(_)
                | PatternKind::Integer(_)
                | PatternKind::Text(_)
                | PatternKind::Tuple(_)
                | PatternKind::List { .. }
                | PatternKind::Record(_) => {}
            }
        }

        for (_, expr) in module.exprs().iter() {
            match &expr.kind {
                ExprKind::Name(reference) => self.consider_site(
                    reference.span(),
                    cursor,
                    NavigationSite::TermReference {
                        name: first_segment_text(&reference.path),
                        resolution: reference.resolution.clone(),
                    },
                    &mut best,
                ),
                ExprKind::SuffixedInteger(literal) => self.consider_site(
                    literal.suffix.span(),
                    cursor,
                    NavigationSite::LiteralSuffix {
                        resolution: literal.resolution.clone(),
                    },
                    &mut best,
                ),
                ExprKind::Binary {
                    left,
                    operator,
                    right,
                } => {
                    if let Some(span) = self.binary_operator_span(*left, *right) {
                        self.consider_site(
                            span,
                            cursor,
                            NavigationSite::BinaryOperator {
                                operator: *operator,
                            },
                            &mut best,
                        );
                    }
                }
                ExprKind::Integer(_)
                | ExprKind::Float(_)
                | ExprKind::Decimal(_)
                | ExprKind::BigInt(_)
                | ExprKind::Text(_)
                | ExprKind::Regex(_)
                | ExprKind::Tuple(_)
                | ExprKind::List(_)
                | ExprKind::Map(_)
                | ExprKind::Set(_)
                | ExprKind::Record(_)
                | ExprKind::AmbientSubject
                | ExprKind::Projection { .. }
                | ExprKind::Apply { .. }
                | ExprKind::Unary { .. }
                | ExprKind::PatchApply { .. }
                | ExprKind::PatchLiteral(_)
                | ExprKind::Pipe(_)
                | ExprKind::Cluster(_)
                | ExprKind::Markup(_) => {}
            }
        }

        best.map(|(_, site)| site)
    }

    fn consider_site(
        &self,
        span: SourceSpan,
        cursor: ByteIndex,
        site: NavigationSite,
        best: &mut Option<(u32, NavigationSite)>,
    ) {
        if span.file() != self.source.id() || !span.span().contains(cursor) {
            return;
        }
        let len = span.span().len();
        if best
            .as_ref()
            .is_none_or(|(current_len, _)| len <= *current_len)
        {
            *best = Some((len, site));
        }
    }

    fn definition_targets_for_site(
        &self,
        db: &RootDatabase,
        site: &NavigationSite,
    ) -> Vec<NavigationTarget> {
        match site {
            NavigationSite::TermReference { name, resolution } => {
                self.definition_targets_for_term_reference(db, name, resolution)
            }
            NavigationSite::TypeReference { name, resolution } => {
                self.definition_targets_for_type_reference(db, name, resolution)
            }
            NavigationSite::LiteralSuffix { resolution } => {
                self.definition_targets_for_literal_suffix(resolution)
            }
            NavigationSite::BindingDecl { binding } => self.binding_targets(*binding),
            NavigationSite::TypeParameterDecl { parameter } => {
                self.type_parameter_targets(*parameter)
            }
            NavigationSite::ImportName { module, import } => {
                self.import_definition_targets(db, module, *import)
            }
            NavigationSite::ExportTarget { name, resolution } => {
                self.export_definition_targets(db, name, resolution)
            }
            NavigationSite::BinaryOperator { operator } => {
                self.binary_operator_definition_targets(*operator)
            }
            NavigationSite::ItemDecl { item } => self.item_targets(*item, None),
            NavigationSite::ConstructorDecl { item, variant_name } => {
                self.item_targets(*item, Some(variant_name.as_ref()))
            }
            NavigationSite::ClassMemberDecl { resolution } => {
                self.class_member_targets(*resolution)
            }
            NavigationSite::DomainMemberDecl { resolution } => {
                self.domain_member_targets(*resolution)
            }
            NavigationSite::InstanceMemberDecl {
                instance,
                member_index,
            } => self.instance_member_targets(*instance, *member_index),
        }
    }

    fn implementation_targets_for_site(
        &self,
        _db: &RootDatabase,
        site: &NavigationSite,
    ) -> Vec<NavigationTarget> {
        match site {
            NavigationSite::TermReference { resolution, .. } => {
                self.implementation_targets_for_term_reference(resolution)
            }
            NavigationSite::TypeReference { resolution, .. } => {
                self.implementation_targets_for_type_reference(resolution)
            }
            NavigationSite::ExportTarget { resolution, .. } => {
                self.implementation_targets_for_export_resolution(resolution)
            }
            NavigationSite::BinaryOperator { operator } => {
                self.binary_operator_implementation_targets(*operator)
            }
            NavigationSite::ItemDecl { item } => self.item_implementation_targets(*item),
            NavigationSite::ClassMemberDecl { resolution } => {
                self.class_member_implementation_targets(*resolution)
            }
            NavigationSite::LiteralSuffix { .. }
            | NavigationSite::BindingDecl { .. }
            | NavigationSite::TypeParameterDecl { .. }
            | NavigationSite::ImportName { .. }
            | NavigationSite::ConstructorDecl { .. }
            | NavigationSite::DomainMemberDecl { .. }
            | NavigationSite::InstanceMemberDecl { .. } => Vec::new(),
        }
    }

    fn definition_targets_for_term_reference(
        &self,
        db: &RootDatabase,
        name: &str,
        resolution: &ResolutionState<TermResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Unresolved => Vec::new(),
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                self.binding_targets(*binding)
            }
            ResolutionState::Resolved(TermResolution::Item(item)) => {
                self.item_targets(*item, Some(name))
            }
            ResolutionState::Resolved(TermResolution::Import(import)) => {
                self.import_definition_targets_for_import_id(db, *import)
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(value)) => {
                self.intrinsic_import_targets(db, name, value.clone())
            }
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => {
                self.domain_member_targets(*resolution)
            }
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates)) => {
                let mut targets = Vec::new();
                for candidate in candidates.iter().copied() {
                    push_targets(&mut targets, self.domain_member_targets(candidate));
                }
                targets
            }
            ResolutionState::Resolved(TermResolution::ClassMember(resolution)) => {
                self.class_member_targets(*resolution)
            }
            ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(candidates)) => {
                let mut targets = Vec::new();
                for candidate in candidates.iter().copied() {
                    push_targets(&mut targets, self.class_member_targets(candidate));
                }
                targets
            }
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                self.builtin_term_import_targets(db, name, *builtin)
            }
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(candidates)) => {
                let mut targets = Vec::new();
                for import_id in candidates.iter().copied() {
                    push_targets(
                        &mut targets,
                        self.import_definition_targets_for_import_id(db, import_id),
                    );
                }
                targets
            }
        }
    }

    fn definition_targets_for_type_reference(
        &self,
        db: &RootDatabase,
        name: &str,
        resolution: &ResolutionState<TypeResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Unresolved => Vec::new(),
            ResolutionState::Resolved(TypeResolution::Item(item)) => {
                self.item_targets(*item, Some(name))
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                self.type_parameter_targets(*parameter)
            }
            ResolutionState::Resolved(TypeResolution::Import(import)) => {
                self.import_definition_targets_for_import_id(db, *import)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
                self.builtin_type_import_targets(db, name, *builtin)
            }
        }
    }

    fn definition_targets_for_literal_suffix(
        &self,
        resolution: &ResolutionState<LiteralSuffixResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Unresolved => Vec::new(),
            ResolutionState::Resolved(resolution) => {
                self.domain_member_targets(DomainMemberResolution {
                    domain: resolution.domain,
                    member_index: resolution.member_index,
                })
            }
        }
    }

    fn export_definition_targets(
        &self,
        db: &RootDatabase,
        name: &str,
        resolution: &ResolutionState<ExportResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Unresolved => Vec::new(),
            ResolutionState::Resolved(ExportResolution::Item(item)) => {
                self.item_targets(*item, Some(name))
            }
            ResolutionState::Resolved(ExportResolution::BuiltinTerm(builtin)) => {
                self.builtin_term_import_targets(db, name, *builtin)
            }
            ResolutionState::Resolved(ExportResolution::BuiltinType(builtin)) => {
                self.builtin_type_import_targets(db, name, *builtin)
            }
            ResolutionState::Resolved(ExportResolution::Import(_)) => Vec::new(),
        }
    }

    fn implementation_targets_for_term_reference(
        &self,
        resolution: &ResolutionState<TermResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Resolved(TermResolution::ClassMember(resolution)) => {
                self.class_member_implementation_targets(*resolution)
            }
            ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(candidates)) => {
                let mut targets = Vec::new();
                for candidate in candidates.iter().copied() {
                    push_targets(
                        &mut targets,
                        self.class_member_implementation_targets(candidate),
                    );
                }
                targets
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_)) => Vec::new(),
        }
    }

    fn implementation_targets_for_type_reference(
        &self,
        resolution: &ResolutionState<TypeResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Resolved(TypeResolution::Item(item)) => {
                self.item_implementation_targets(*item)
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_)) => Vec::new(),
        }
    }

    fn implementation_targets_for_export_resolution(
        &self,
        resolution: &ResolutionState<ExportResolution>,
    ) -> Vec<NavigationTarget> {
        match resolution {
            ResolutionState::Resolved(ExportResolution::Item(item)) => {
                self.item_implementation_targets(*item)
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(ExportResolution::BuiltinTerm(_))
            | ResolutionState::Resolved(ExportResolution::BuiltinType(_))
            | ResolutionState::Resolved(ExportResolution::Import(_)) => Vec::new(),
        }
    }

    fn binding_targets(&self, binding: BindingId) -> Vec<NavigationTarget> {
        self.module()
            .bindings()
            .get(binding)
            .map(|binding| vec![NavigationTarget::new(self.file, binding.name.span())])
            .unwrap_or_default()
    }

    fn type_parameter_targets(&self, parameter: TypeParameterId) -> Vec<NavigationTarget> {
        self.module()
            .type_parameters()
            .get(parameter)
            .map(|parameter| vec![NavigationTarget::new(self.file, parameter.name.span())])
            .unwrap_or_default()
    }

    fn item_targets(&self, item: ItemId, referenced_name: Option<&str>) -> Vec<NavigationTarget> {
        self.item_selection_span(item, referenced_name)
            .map(|span| vec![NavigationTarget::new(self.file, span)])
            .unwrap_or_default()
    }

    fn class_member_targets(&self, resolution: ClassMemberResolution) -> Vec<NavigationTarget> {
        self.class_member_selection_span(resolution)
            .map(|span| vec![NavigationTarget::new(self.file, span)])
            .unwrap_or_default()
    }

    fn domain_member_targets(&self, resolution: DomainMemberResolution) -> Vec<NavigationTarget> {
        self.domain_member_selection_span(resolution)
            .map(|span| vec![NavigationTarget::new(self.file, span)])
            .unwrap_or_default()
    }

    fn instance_member_targets(
        &self,
        instance: ItemId,
        member_index: usize,
    ) -> Vec<NavigationTarget> {
        self.instance_member_selection_span(instance, member_index)
            .map(|span| vec![NavigationTarget::new(self.file, span)])
            .unwrap_or_default()
    }

    fn item_selection_span(
        &self,
        item: ItemId,
        referenced_name: Option<&str>,
    ) -> Option<SourceSpan> {
        match self.module().items().get(item)? {
            Item::Type(item) => {
                if let Some(referenced_name) = referenced_name
                    && let TypeItemBody::Sum(variants) = &item.body
                    && let Some(variant) = variants
                        .iter()
                        .find(|variant| variant.name.text() == referenced_name)
                {
                    return Some(variant.name.span());
                }
                Some(item.name.span())
            }
            Item::Value(item) => Some(item.name.span()),
            Item::Function(item) => Some(item.name.span()),
            Item::Signal(item) => Some(item.name.span()),
            Item::Class(item) => Some(item.name.span()),
            Item::Domain(item) => Some(item.name.span()),
            Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_)
            | Item::SourceProviderContract(_) => None,
        }
    }

    fn class_member_selection_span(&self, resolution: ClassMemberResolution) -> Option<SourceSpan> {
        let Item::Class(class_item) = self.module().items().get(resolution.class)? else {
            return None;
        };
        Some(class_item.members.get(resolution.member_index)?.name.span())
    }

    fn domain_member_selection_span(
        &self,
        resolution: DomainMemberResolution,
    ) -> Option<SourceSpan> {
        let Item::Domain(domain_item) = self.module().items().get(resolution.domain)? else {
            return None;
        };
        Some(
            domain_item
                .members
                .get(resolution.member_index)?
                .name
                .span(),
        )
    }

    fn instance_member_selection_span(
        &self,
        instance: ItemId,
        member_index: usize,
    ) -> Option<SourceSpan> {
        let Item::Instance(instance_item) = self.module().items().get(instance)? else {
            return None;
        };
        Some(instance_item.members.get(member_index)?.name.span())
    }

    fn import_definition_targets_for_import_id(
        &self,
        db: &RootDatabase,
        import: ImportId,
    ) -> Vec<NavigationTarget> {
        let Some(module) = self.module_path_for_import(import) else {
            return Vec::new();
        };
        self.import_definition_targets(db, &module, import)
    }

    fn import_definition_targets(
        &self,
        db: &RootDatabase,
        module: &NamePath,
        import: ImportId,
    ) -> Vec<NavigationTarget> {
        let Some(import_binding) = self.module().imports().get(import) else {
            return Vec::new();
        };
        if import_binding.resolution != ImportBindingResolution::Resolved {
            return Vec::new();
        }

        let module_segments = module
            .segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>();
        let Some(target_file) = aivi_query::resolve_module_file(db, self.file, &module_segments)
        else {
            return Vec::new();
        };
        let imported = Self::load(db, target_file);
        match &import_binding.metadata {
            ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::Domain { .. }
            | ImportBindingMetadata::BuiltinType(_)
            | ImportBindingMetadata::AmbientType => {
                imported.type_declaration_targets(import_binding.imported_name.text())
            }
            ImportBindingMetadata::Value { .. }
            | ImportBindingMetadata::IntrinsicValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::BuiltinTerm(_) => {
                imported.term_declaration_targets(import_binding.imported_name.text())
            }
            ImportBindingMetadata::Bundle(_) => {
                let mut targets =
                    imported.type_declaration_targets(import_binding.imported_name.text());
                push_targets(
                    &mut targets,
                    imported.term_declaration_targets(import_binding.imported_name.text()),
                );
                targets
            }
            ImportBindingMetadata::Unknown | ImportBindingMetadata::InstanceMember { .. } => {
                Vec::new()
            }
        }
    }

    fn term_declaration_targets(&self, name: &str) -> Vec<NavigationTarget> {
        let mut targets = Vec::new();
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            match item {
                Item::Type(item) => {
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            if variant.name.text() == name {
                                push_target(
                                    &mut targets,
                                    Some(NavigationTarget::new(self.file, variant.name.span())),
                                );
                            }
                        }
                    }
                }
                Item::Value(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Function(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Signal(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Class(_)
                | Item::Domain(_)
                | Item::Value(_)
                | Item::Function(_)
                | Item::Signal(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => {}
            }
        }
        targets
    }

    fn type_declaration_targets(&self, name: &str) -> Vec<NavigationTarget> {
        let mut targets = Vec::new();
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            match item {
                Item::Type(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Class(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Domain(item) if item.name.text() == name => {
                    push_target(
                        &mut targets,
                        Some(NavigationTarget::new(self.file, item.name.span())),
                    );
                }
                Item::Value(_)
                | Item::Function(_)
                | Item::Signal(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_)
                | Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_) => {}
            }
        }
        targets
    }

    fn module_path_for_import(&self, import: ImportId) -> Option<NamePath> {
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            let Item::Use(use_item) = item else {
                continue;
            };
            if use_item
                .imports
                .iter()
                .any(|candidate| *candidate == import)
            {
                return Some(use_item.module.clone());
            }
        }
        None
    }

    fn find_unique_import_site(
        &self,
        mut matches: impl FnMut(&ImportBinding) -> bool,
    ) -> Option<(NamePath, ImportId)> {
        let mut matched = None;
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            let Item::Use(use_item) = item else {
                continue;
            };
            for import_id in use_item.imports.iter().copied() {
                let import = &self.module().imports()[import_id];
                if !matches(import) {
                    continue;
                }
                if matched.is_some() {
                    return None;
                }
                matched = Some((use_item.module.clone(), import_id));
            }
        }
        matched
    }

    fn builtin_term_import_targets(
        &self,
        db: &RootDatabase,
        local_name: &str,
        builtin: BuiltinTerm,
    ) -> Vec<NavigationTarget> {
        let Some((module, import)) = self.find_unique_import_site(|import| {
            import.local_name.text() == local_name
                && matches!(
                    import.metadata,
                    ImportBindingMetadata::BuiltinTerm(candidate) if candidate == builtin
                )
        }) else {
            return Vec::new();
        };
        self.import_definition_targets(db, &module, import)
    }

    fn builtin_type_import_targets(
        &self,
        db: &RootDatabase,
        local_name: &str,
        builtin: BuiltinType,
    ) -> Vec<NavigationTarget> {
        let Some((module, import)) = self.find_unique_import_site(|import| {
            import.local_name.text() == local_name
                && matches!(
                    import.metadata,
                    ImportBindingMetadata::BuiltinType(candidate) if candidate == builtin
                )
        }) else {
            return Vec::new();
        };
        self.import_definition_targets(db, &module, import)
    }

    fn intrinsic_import_targets(
        &self,
        db: &RootDatabase,
        local_name: &str,
        value: aivi_hir::IntrinsicValue,
    ) -> Vec<NavigationTarget> {
        let Some((module, import)) = self.find_unique_import_site(|import| {
            import.local_name.text() == local_name
                && matches!(
                    &import.metadata,
                    ImportBindingMetadata::IntrinsicValue { value: candidate, .. }
                        if *candidate == value
                )
        }) else {
            return Vec::new();
        };
        self.import_definition_targets(db, &module, import)
    }

    fn binary_operator_span(
        &self,
        left: aivi_hir::ExprId,
        right: aivi_hir::ExprId,
    ) -> Option<SourceSpan> {
        let left_span = self.module().exprs().get(left)?.span;
        let right_span = self.module().exprs().get(right)?.span;
        if left_span.file() != self.source.id()
            || right_span.file() != self.source.id()
            || left_span.file() != right_span.file()
        {
            return None;
        }

        let start = left_span.span().end().as_usize();
        let end = right_span.span().start().as_usize();
        if start >= end || end > self.source.len() {
            return None;
        }

        let between = &self.source.text()[start..end];
        let trimmed = between.trim();
        if trimmed.is_empty() {
            return None;
        }

        let leading = between.len() - between.trim_start().len();
        let trailing = between.len() - between.trim_end().len();
        let operator_start = start + leading;
        let operator_end = end - trailing;
        if operator_start >= operator_end {
            return None;
        }

        Some(SourceSpan::new(
            self.source.id(),
            Span::from(operator_start..operator_end),
        ))
    }

    fn binary_operator_definition_targets(
        &self,
        operator: BinaryOperator,
    ) -> Vec<NavigationTarget> {
        let mut targets = Vec::new();
        let operator_text = binary_operator_text(operator);
        for item_id in self.module().root_items().iter().copied() {
            match &self.module().items()[item_id] {
                Item::Class(class_item) => {
                    for (member_index, member) in class_item.members.iter().enumerate() {
                        if member.name.text() == operator_text {
                            push_targets(
                                &mut targets,
                                self.class_member_targets(ClassMemberResolution {
                                    class: item_id,
                                    member_index,
                                }),
                            );
                        }
                    }
                }
                Item::Domain(domain_item) => {
                    for (member_index, member) in domain_item.members.iter().enumerate() {
                        if member.kind == DomainMemberKind::Operator
                            && member.name.text() == operator_text
                        {
                            push_targets(
                                &mut targets,
                                self.domain_member_targets(DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                }),
                            );
                        }
                    }
                }
                Item::Type(_)
                | Item::Value(_)
                | Item::Function(_)
                | Item::Signal(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => {}
            }
        }
        targets
    }

    fn binary_operator_implementation_targets(
        &self,
        operator: BinaryOperator,
    ) -> Vec<NavigationTarget> {
        let mut targets = Vec::new();
        let operator_text = binary_operator_text(operator);
        for item_id in self.module().root_items().iter().copied() {
            let Item::Class(class_item) = &self.module().items()[item_id] else {
                continue;
            };
            for (member_index, member) in class_item.members.iter().enumerate() {
                if member.name.text() == operator_text {
                    push_targets(
                        &mut targets,
                        self.class_member_implementation_targets(ClassMemberResolution {
                            class: item_id,
                            member_index,
                        }),
                    );
                }
            }
        }
        targets
    }

    fn item_implementation_targets(&self, item: ItemId) -> Vec<NavigationTarget> {
        match self.module().items().get(item) {
            Some(Item::Class(_)) => self.class_instance_targets(item),
            Some(
                Item::Type(_)
                | Item::Value(_)
                | Item::Function(_)
                | Item::Signal(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_),
            )
            | None => Vec::new(),
        }
    }

    fn class_instance_targets(&self, class_item: ItemId) -> Vec<NavigationTarget> {
        let mut targets = Vec::new();
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            let Item::Instance(instance) = item else {
                continue;
            };
            if matches!(
                instance.class.resolution,
                ResolutionState::Resolved(TypeResolution::Item(resolved)) if resolved == class_item
            ) {
                push_target(
                    &mut targets,
                    Some(NavigationTarget::new(self.file, instance.class.span())),
                );
            }
        }
        targets
    }

    fn class_member_implementation_targets(
        &self,
        resolution: ClassMemberResolution,
    ) -> Vec<NavigationTarget> {
        let Some(Item::Class(class_item)) = self.module().items().get(resolution.class) else {
            return Vec::new();
        };
        let Some(member) = class_item.members.get(resolution.member_index) else {
            return Vec::new();
        };
        let member_name = member.name.text();
        let mut targets = Vec::new();
        for item_id in self.module().root_items().iter().copied() {
            let item = &self.module().items()[item_id];
            let Item::Instance(instance) = item else {
                continue;
            };
            if !matches!(
                instance.class.resolution,
                ResolutionState::Resolved(TypeResolution::Item(resolved)) if resolved == resolution.class
            ) {
                continue;
            }
            if let Some(member_index) = instance
                .members
                .iter()
                .position(|candidate| candidate.name.text() == member_name)
            {
                push_targets(
                    &mut targets,
                    self.instance_member_targets(item_id, member_index),
                );
            }
        }
        targets
    }
}

pub fn goto_response(
    db: &RootDatabase,
    targets: Vec<NavigationTarget>,
) -> Option<GotoDefinitionResponse> {
    let mut locations = Vec::new();
    for target in targets {
        let Some(location) = location_for_target(db, target) else {
            continue;
        };
        if !locations.contains(&location) {
            locations.push(location);
        }
    }

    match locations.len() {
        0 => None,
        1 => Some(GotoDefinitionResponse::Scalar(
            locations.into_iter().next().expect("length checked above"),
        )),
        _ => Some(GotoDefinitionResponse::Array(locations)),
    }
}

fn location_for_target(db: &RootDatabase, target: NavigationTarget) -> Option<Location> {
    let uri = Url::from_file_path(target.file.path(db)).ok()?;
    let source = target.file.source(db);
    let range = crate::diagnostics::lsp_range(source.span_to_lsp_range(target.span.span()));
    Some(Location { uri, range })
}

fn first_segment_text(path: &NamePath) -> Box<str> {
    path.segments().first().text().to_owned().into_boxed_str()
}

fn push_targets(targets: &mut Vec<NavigationTarget>, additional: Vec<NavigationTarget>) {
    for target in additional {
        push_target(targets, Some(target));
    }
}

fn push_target(targets: &mut Vec<NavigationTarget>, target: Option<NavigationTarget>) {
    let Some(target) = target else {
        return;
    };
    if !targets.contains(&target) {
        targets.push(target);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NavigationSite {
    TermReference {
        name: Box<str>,
        resolution: ResolutionState<TermResolution>,
    },
    TypeReference {
        name: Box<str>,
        resolution: ResolutionState<TypeResolution>,
    },
    LiteralSuffix {
        resolution: ResolutionState<LiteralSuffixResolution>,
    },
    BindingDecl {
        binding: BindingId,
    },
    TypeParameterDecl {
        parameter: TypeParameterId,
    },
    ImportName {
        module: NamePath,
        import: ImportId,
    },
    ExportTarget {
        name: Box<str>,
        resolution: ResolutionState<ExportResolution>,
    },
    BinaryOperator {
        operator: BinaryOperator,
    },
    ItemDecl {
        item: ItemId,
    },
    ConstructorDecl {
        item: ItemId,
        variant_name: Box<str>,
    },
    ClassMemberDecl {
        resolution: ClassMemberResolution,
    },
    DomainMemberDecl {
        resolution: DomainMemberResolution,
    },
    InstanceMemberDecl {
        instance: ItemId,
        member_index: usize,
    },
}

fn binary_operator_text(operator: BinaryOperator) -> &'static str {
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
