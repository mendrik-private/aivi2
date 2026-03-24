use std::fs;
use aivi_base::SourceDatabase;
use aivi_syntax::parse_module;
use aivi_hir::{lower_module, Item};

fn main() {
    let text = fs::read_to_string("demos/snake.aivi").expect("read snake.aivi");
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/snake.aivi", &text);
    let parsed = parse_module(&sources[file_id]);
    
    if parsed.has_errors() {
        eprintln!("Parse errors: {:?}", parsed.all_diagnostics().collect::<Vec<_>>());
        return;
    }
    
    let hir_result = lower_module(&parsed.module);
    
    if hir_result.has_errors() {
        eprintln!("HIR errors: {:?}", hir_result.hir_diagnostics());
        return;
    }
    
    let module = hir_result.module();
    
    println!("All items by ID:");
    for item_id in module.items().iter_ids() {
        let item = &module.items()[item_id];
        let id_num = item_id.as_raw();
        let kind_str = match item {
            Item::Type(t) => format!("Type: {}", t.name.text()),
            Item::Value(v) => format!("Value: {}", v.name.text()),
            Item::Function(f) => format!("Function: {}", f.name.text()),
            Item::Signal(s) => format!("Signal: {}", s.name.text()),
            Item::Class(c) => format!("Class: {}", c.name.text()),
            Item::Domain(d) => format!("Domain: {}", d.name.text()),
            Item::SourceProviderContract(spc) => format!("SourceProviderContract: {}", spc.name.text()),
            Item::Instance(i) => format!("Instance: {}", i.name.text()),
            Item::Use(_) => "Use".to_string(),
            Item::Export(_) => "Export".to_string(),
        };
        println!("{:3}: {}", id_num, kind_str);
    }
}
