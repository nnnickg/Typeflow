use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use typeflow_core::data::LanguageBundle;
use typeflow_core::{Action, Engine, EngineConfig, InputEvent, Layout, LetterEvent, PhysicalKey};

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
    group.bench_function("process_action_mixed_physical_batch", |b| {
        let mut engine = engine();
        b.iter(|| feed_physical_batch(&mut engine, black_box(MIXED_TOKENS)));
    });

    group.throughput(Throughput::Elements(WRONG_LAYOUT_TOKEN.len() as u64));
    group.bench_function("process_full_output_wrong_layout_token", |b| {
        let mut engine = engine();
        b.iter(|| feed_full_output_token(&mut engine, black_box(WRONG_LAYOUT_TOKEN)));
    });

    for len in [64usize, 256, 1024] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_with_input(
            BenchmarkId::new("process_action_letter_run", len),
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
    let mut committed = String::new();
    for token in tokens {
        engine.reset_layout(Layout::English);
        committed.clear();
        for key in *token {
            let action = engine.process_action(InputEvent::Letter(LetterEvent::new(*key)));
            apply_action(&action, &mut committed);
            black_box(&committed);
        }
        black_box(engine.current_layout());
    }
}

fn feed_full_output_token(engine: &mut Engine, token: &[PhysicalKey]) {
    let mut committed = String::new();
    engine.reset_layout(Layout::English);
    for key in token {
        let output = engine.process(InputEvent::Letter(LetterEvent::new(*key)));
        apply_action(&output.action, &mut committed);
        black_box(output);
        black_box(&committed);
    }
    black_box(engine.current_layout());
}

fn feed_letter_run(engine: &mut Engine, len: usize) {
    let mut committed = String::new();
    engine.reset_layout(Layout::English);
    for _ in 0..len {
        let action = engine.process_action(InputEvent::Letter(LetterEvent::new(PhysicalKey::A)));
        apply_action(&action, &mut committed);
    }
    black_box(engine.token_len());
    black_box(committed);
}

fn apply_action(action: &Action, committed: &mut String) {
    match action {
        Action::Keep | Action::ResetToken => {}
        Action::Commit(character) => committed.push(*character),
        Action::ReplaceToken {
            old_len,
            replacement,
            ..
        } => {
            for _ in 0..*old_len {
                committed.pop();
            }
            committed.push_str(replacement);
        }
    }
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
