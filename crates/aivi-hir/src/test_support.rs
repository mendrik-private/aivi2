use std::{fs, path::PathBuf};

use aivi_base::SourceDatabase;
use aivi_syntax::parse_module;

use crate::{
    ImportModuleResolution, ImportResolver, Item, ItemId, Module, exports, lower_module,
    lower_module_with_resolver,
};

pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

pub fn lower_text(path: &str, text: &str) -> crate::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    lower_module(&parsed.module)
}

/// A test-only [`ImportResolver`] that resolves `aivi.*` stdlib modules by
/// reading from the bundled stdlib directory at `../../stdlib` relative to the
/// crate manifest.
struct StdlibResolver {
    stdlib_root: PathBuf,
}

impl StdlibResolver {
    fn new() -> Self {
        let stdlib_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stdlib");
        Self { stdlib_root }
    }
}

impl ImportResolver for StdlibResolver {
    fn resolve(&self, path: &[&str]) -> ImportModuleResolution {
        if path.first() != Some(&"aivi") {
            return ImportModuleResolution::Missing;
        }
        let mut file_path = self.stdlib_root.clone();
        for segment in path {
            file_path.push(segment);
        }
        file_path.set_extension("aivi");
        let text = match fs::read_to_string(&file_path) {
            Ok(t) => t,
            Err(_) => return ImportModuleResolution::Missing,
        };
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(file_path.to_string_lossy().as_ref(), text.as_str());
        let parsed = parse_module(&sources[file_id]);
        if parsed.has_errors() {
            return ImportModuleResolution::Missing;
        }
        let lowered = lower_module(&parsed.module);
        ImportModuleResolution::Resolved(exports(lowered.module()))
    }
}

/// Like [`lower_text`], but resolves `aivi.*` stdlib imports from the bundled
/// stdlib on disk.  Use for tests that include files which import stdlib modules.
pub fn lower_text_with_stdlib(path: &str, text: &str) -> crate::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let resolver = StdlibResolver::new();
    lower_module_with_resolver(&parsed.module, Some(&resolver))
}

pub fn lower_fixture(path: &str) -> crate::LoweringResult {
    let text = fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
    lower_text(path, &text)
}

pub fn item_name(module: &Module, item_id: ItemId) -> &str {
    match &module.items()[item_id] {
        Item::Type(item) => item.name.text(),
        Item::Value(item) => item.name.text(),
        Item::Function(item) => item.name.text(),
        Item::Signal(item) => item.name.text(),
        Item::Class(item) => item.name.text(),
        Item::Domain(item) => item.name.text(),
        Item::SourceProviderContract(_) | Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::Hoist(_) => {
            "<anonymous>"
        }
    }
}
