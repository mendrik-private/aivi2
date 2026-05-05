use criterion::{Criterion, black_box, criterion_group, criterion_main};
use aivi_base::SourceDatabase;
use aivi_syntax::{lex_module, parse_module};

fn bench_parse_snake(c: &mut Criterion) {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/snake.aivi", include_str!("../../demos/snake.aivi"));
    let source_file = &sources[file_id];
    c.bench_function("parse_snake", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
            let _module = parse_module(black_box(source_file));
        })
    });
}

fn bench_parse_reversi(c: &mut Criterion) {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("demos/reversi.aivi", include_str!("../../demos/reversi.aivi"));
    let source_file = &sources[file_id];
    c.bench_function("parse_reversi", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
            let _module = parse_module(black_box(source_file));
        })
    });
}

fn bench_lex_large_source(c: &mut Criterion) {
    let mut sources = SourceDatabase::new();
    let source = include_str!("../../demos/snake.aivi").repeat(10);
    let file_id = sources.add_file("demos/snake_10x.aivi", source);
    let source_file = &sources[file_id];
    c.bench_function("lex_10x_snake", |b| {
        b.iter(|| {
            let _tokens = lex_module(black_box(source_file));
        })
    });
}

criterion_group!(parser, bench_parse_snake, bench_parse_reversi, bench_lex_large_source);
criterion_main!(parser);
