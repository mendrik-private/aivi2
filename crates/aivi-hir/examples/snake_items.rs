use std::fs;
use std::sync::Arc;
use aivi_base::SourceDatabase;
use aivi_syntax::parse_module;
use aivi_hir::{lower_module_with_resolver, Item, NullImportResolver};

fn main() {
    let text = fs::read_to_string("demos/snake.aivi").expect("read snake.aivi");
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/snake.aivi", Arc::from(text.as_str()));
    let parsed = parse_module(&sources[file_id]);
    
    if parsed.has_errors() {
        eprintln!("Parse errors: {:?}", parsed.all_diagnostics().collect::<Vec<_>>());
        return;
    }
    
    // Create a resolver
    let resolver = NullImportResolver;
    let hir_result = lower_module_with_resolver(&parsed.module, Some(&resolver));
    
    if hir_result.has_errors() {
        eprintln!("HIR has errors, but proceeding anyway");
        for diag in hir_result.diagnostics() {
            eprintln!("  {}", diag.message);
        }
    }
    
    let module = hir_result.module();
    
    println!("Items by ID (in order):");
    for (item_id, item) in module.items().iter() {
        let id_num = item_id.as_raw();
        let kind_str = match item {
            Item::Type(t) => format!("Type: {}", t.name.text()),
            Item::Value(v) => format!("Value: {}", v.name.text()),
            Item::Function(f) => format!("Function: {}", f.name.text()),
            Item::Signal(s) => format!("Signal: {}", s.name.text()),
            Item::Class(c) => format!("Class: {}", c.name.text()),
            Item::Domain(d) => format!("Domain: {}", d.name.text()),
            Item::SourceProviderContract(_spc) => "SourceProviderContract".to_string(),
            Item::Instance(_i) => "Instance".to_string(),
            Item::Use(_) => "Use".to_string(),
            Item::Export(_) => "Export".to_string(),
        };
        println!("{:3}: {}", id_num, kind_str);
    }
}
