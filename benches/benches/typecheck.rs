use std::{fs, path::PathBuf};

use aivi_base::SourceDatabase;
use aivi_hir::{
    ImportModuleResolution, ImportResolver, typecheck_module,
    lower_module, lower_module_with_resolver, exports,
};
use aivi_syntax::{lex_module, parse_module};

use criterion::{Criterion, black_box, criterion_group, criterion_main};

/// Resolves `aivi.*` stdlib imports from the bundled stdlib directory.
struct StdlibResolver {
    stdlib_root: PathBuf,
}

impl StdlibResolver {
    fn new() -> Self {
        Self { stdlib_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib") }
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

fn bench_typecheck_snake(c: &mut Criterion) {
    let resolver = StdlibResolver::new();
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/snake.aivi", include_str!("../../demos/snake.aivi"));
    let source_file = &sources[file_id];
    c.bench_function("typecheck_snake", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
            let parsed = parse_module(black_box(source_file));
            let lowered = lower_module_with_resolver(black_box(&parsed.module), Some(&resolver));
            let _report = typecheck_module(lowered.module());
        })
    });
}

fn bench_typecheck_reversi(c: &mut Criterion) {
    let resolver = StdlibResolver::new();
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/reversi.aivi", include_str!("../../demos/reversi.aivi"));
    let source_file = &sources[file_id];
    c.bench_function("typecheck_reversi", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
            let parsed = parse_module(black_box(source_file));
            let lowered = lower_module_with_resolver(black_box(&parsed.module), Some(&resolver));
            let _report = typecheck_module(lowered.module());
        })
    });
}

fn bench_typecheck_large(c: &mut Criterion) {
    let resolver = StdlibResolver::new();
    let mut sources = SourceDatabase::new();
    let source = include_str!("../../demos/snake.aivi").repeat(10);
    let file_id = sources.add_file("demos/snake_10x.aivi", source);
    let source_file = &sources[file_id];
    c.bench_function("typecheck_10x_snake", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
            let parsed = parse_module(black_box(source_file));
            let lowered = lower_module_with_resolver(black_box(&parsed.module), Some(&resolver));
            let _report = typecheck_module(lowered.module());
        })
    });
}

criterion_group!(typecheck, bench_typecheck_snake, bench_typecheck_reversi, bench_typecheck_large);
criterion_main!(typecheck);
