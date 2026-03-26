use std::{fs, sync::Arc, thread, time::Duration};
use aivi_base::SourceDatabase;
use aivi_hir::{Item, lower_module as lower_hir_module};
use aivi_lambda::lower_module as lower_lambda_module;
use aivi_runtime::{SourceProviderManager, assemble_hir_runtime, link_backend_runtime};
use aivi_runtime::providers::WindowKeyEvent;
use aivi_syntax::parse_module;

fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
    module.items().iter().find_map(|(item_id, item)| match item {
        Item::Value(item) if item.name.text() == name => Some(item_id),
        Item::Function(item) if item.name.text() == name => Some(item_id),
        Item::Signal(item) if item.name.text() == name => Some(item_id),
        Item::Type(item) if item.name.text() == name => Some(item_id),
        Item::Class(item) if item.name.text() == name => Some(item_id),
        Item::Domain(item) if item.name.text() == name => Some(item_id),
        _ => None,
    }).unwrap()
}

fn main() {
    let path = "/home/mendrik/desk/mendrik/aivi2/demos/snake.aivi";
    let source = fs::read_to_string(path).unwrap();
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, source);
    let parsed = parse_module(&sources[file_id]);
    let hir = lower_hir_module(&parsed.module);
    let core = aivi_core::lower_runtime_module(hir.module()).unwrap();
    let lambda = lower_lambda_module(&core).unwrap();
    let backend = aivi_backend::lower_module(&lambda).unwrap();
    let assembly = assemble_hir_runtime(hir.module()).unwrap();
    let mut linked = link_backend_runtime(assembly, &core, Arc::new(backend)).unwrap();
    let mut providers = SourceProviderManager::new();
    let first = linked.tick_with_source_lifecycle().unwrap();
    providers.apply_actions(first.source_actions()).unwrap();
    providers.dispatch_window_key_event(WindowKeyEvent { name: "ArrowDown".into(), repeated: false });
    thread::sleep(Duration::from_millis(40));
    let second = linked.tick_with_source_lifecycle().unwrap();
    providers.apply_actions(second.source_actions()).unwrap();
    let game_signal = linked.assembly().signal(item_id(hir.module(), "game")).unwrap().signal();
    println!("{}", linked.runtime().current_value(game_signal).unwrap().unwrap());
}
