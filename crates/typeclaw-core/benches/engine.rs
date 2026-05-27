#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use typeclaw_core::data::LanguageBundle;
use typeclaw_core::{Engine, EngineConfig, InputEvent, Layout, LetterEvent, PhysicalKey};

const MIXED_TOKENS: &[&[PhysicalKey]] = &[
    &[
        PhysicalKey::G,
        PhysicalKey::H,
        PhysicalKey::S,
        PhysicalKey::D,
        PhysicalKey::B,
        PhysicalKey::N,
    ],
    &[
        PhysicalKey::T,
        PhysicalKey::Y,
        PhysicalKey::P,
        PhysicalKey::E,
        PhysicalKey::F,
        PhysicalKey::L,
        PhysicalKey::O,
        PhysicalKey::W,
    ],
    &[
        PhysicalKey::H,
        PhysicalKey::T,
        PhysicalKey::T,
        PhysicalKey::P,
    ],
    &[
        PhysicalKey::K,
        PhysicalKey::U,
        PhysicalKey::B,
        PhysicalKey::E,
        PhysicalKey::C,
        PhysicalKey::T,
        PhysicalKey::L,
    ],
    &[
        PhysicalKey::S,
        PhysicalKey::N,
        PhysicalKey::A,
        PhysicalKey::K,
        PhysicalKey::E,
    ],
    &[
        PhysicalKey::C,
        PhysicalKey::A,
        PhysicalKey::M,
        PhysicalKey::E,
        PhysicalKey::L,
    ],
];

const WRONG_LAYOUT_TOKEN: &[PhysicalKey] = &[
    PhysicalKey::G,
    PhysicalKey::H,
    PhysicalKey::S,
    PhysicalKey::D,
    PhysicalKey::B,
    PhysicalKey::N,
];

fn bench_engine(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine");
    group.throughput(Throughput::Elements(batch_key_count(MIXED_TOKENS) as u64));
    group.bench_function("observe_mixed_physical_batch", |b| {
        let mut engine = engine();
        b.iter(|| feed_physical_batch(&mut engine, black_box(MIXED_TOKENS)));
    });

    group.throughput(Throughput::Elements(WRONG_LAYOUT_TOKEN.len() as u64));
    group.bench_function("observe_full_output_wrong_layout_token", |b| {
        let mut engine = engine();
        b.iter(|| feed_full_output_token(&mut engine, black_box(WRONG_LAYOUT_TOKEN)));
    });

    for len in [64usize, 256, 1024] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_with_input(
            BenchmarkId::new("observe_letter_run", len),
            &len,
            |b, len| {
                let mut engine = engine();
                b.iter(|| feed_letter_run(&mut engine, black_box(*len)));
            },
        );
    }

    group.finish();
}

fn feed_physical_batch(engine: &mut Engine, tokens: &[&[PhysicalKey]]) {
    for token in tokens {
        engine.reset_layout(Layout::English);
        for key in *token {
            let action = engine
                .observe(InputEvent::Letter(LetterEvent::new(*key)))
                .action;
            black_box(action);
        }
        black_box(engine.current_layout());
    }
}

fn feed_full_output_token(engine: &mut Engine, token: &[PhysicalKey]) {
    engine.reset_layout(Layout::English);
    for key in token {
        let output = engine.observe(InputEvent::Letter(LetterEvent::new(*key)));
        black_box(output);
    }
    black_box(engine.current_layout());
}

fn feed_letter_run(engine: &mut Engine, len: usize) {
    engine.reset_layout(Layout::English);
    for _ in 0..len {
        let action = engine
            .observe(InputEvent::Letter(LetterEvent::new(PhysicalKey::A)))
            .action;
        black_box(action);
    }
    black_box(engine.token_len());
}

fn batch_key_count(tokens: &[&[PhysicalKey]]) -> usize {
    tokens.iter().map(|token| token.len()).sum()
}

fn engine() -> Engine {
    Engine::new(
        EngineConfig::default(),
        LanguageBundle::embedded().expect("embedded language bundle should load"),
    )
}

criterion_group!(benches, bench_engine);
criterion_main!(benches);
