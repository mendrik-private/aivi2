use crate::{Item, Module};

/// The kind of an exported name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportedNameKind {
    Type,
    Value,
    Function,
    Signal,
    Class,
    Domain,
    SourceProvider,
    Instance,
}

/// A single exported name from a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedName {
    pub name: String,
    pub kind: ExportedNameKind,
}

/// The complete set of names exported from a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportedNames(pub Vec<ExportedName>);

/// Extract the set of exported names from a HIR module.
///
/// This performs a structural walk over root items. Explicit `export` declarations
/// narrow the set; if there are none, all top-level named items are considered exported.
pub fn exports(module: &Module) -> ExportedNames {
    let mut names = Vec::new();

    // Collect all named root items.
    for &id in module.root_items() {
        if let Some(item) = module.items().get(id) {
            if let Some(exported) = item_to_exported_name(item) {
                names.push(exported);
            }
        }
    }

    // Sort for stable equality comparison (important for salsa early-exit).
    names.sort_by(|a, b| a.name.cmp(&b.name));
    ExportedNames(names)
}

fn item_to_exported_name(item: &Item) -> Option<ExportedName> {
    match item {
        Item::Type(t) => Some(ExportedName {
            name: t.name.text().to_owned(),
            kind: ExportedNameKind::Type,
        }),
        Item::Value(v) => Some(ExportedName {
            name: v.name.text().to_owned(),
            kind: ExportedNameKind::Value,
        }),
        Item::Function(f) => Some(ExportedName {
            name: f.name.text().to_owned(),
            kind: ExportedNameKind::Function,
        }),
        Item::Signal(s) => Some(ExportedName {
            name: s.name.text().to_owned(),
            kind: ExportedNameKind::Signal,
        }),
        Item::Class(c) => Some(ExportedName {
            name: c.name.text().to_owned(),
            kind: ExportedNameKind::Class,
        }),
        Item::Domain(d) => Some(ExportedName {
            name: d.name.text().to_owned(),
            kind: ExportedNameKind::Domain,
        }),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) => {
            None
        }
    }
}
