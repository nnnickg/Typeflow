use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use typeflow_ffi::{
    TF_ACTION_COMMIT, TF_ACTION_KEEP, TF_ACTION_REPLACE, TF_ACTION_RESET, TF_EVENT_LETTER,
    TF_LAYOUT_ENGLISH, TF_REPLACE_BUF_LEN, TfAction, TfEvent, typeflow_engine_free,
    typeflow_engine_new_embedded, typeflow_engine_process, typeflow_engine_reset_layout,
};

const MIXED_TOKENS: &[&[u8]] = &[
    &[6, 7, 18, 3, 1, 13],
    &[19, 24, 15, 4, 5, 11, 14, 22],
    &[7, 19, 19, 15],
    &[10, 20, 1, 4, 2, 19, 11],
    &[18, 13, 0, 10, 4],
    &[2, 0, 12, 4, 11],
];

const WRONG_LAYOUT_TOKEN: &[u8] = &[6, 7, 18, 3, 1, 13];

fn bench_ffi(c: &mut Criterion) {
    let mut group = c.benchmark_group("ffi");

    group.throughput(Throughput::Elements(batch_key_count(MIXED_TOKENS) as u64));
    group.bench_function("process_mixed_physical_batch", |b| {
        let engine = typeflow_engine_new_embedded();
        assert!(!engine.is_null());
        b.iter(|| feed_process_batch(engine, MIXED_TOKENS));
        unsafe {
            typeflow_engine_free(engine);
        }
    });

    group.throughput(Throughput::Elements(WRONG_LAYOUT_TOKEN.len() as u64));
    group.bench_function("process_and_apply_wrong_layout_token", |b| {
        let engine = typeflow_engine_new_embedded();
        assert!(!engine.is_null());
        b.iter(|| feed_and_apply_token(engine, WRONG_LAYOUT_TOKEN));
        unsafe {
            typeflow_engine_free(engine);
        }
    });

    for len in [64usize, 256, 1024] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_with_input(
            BenchmarkId::new("process_letter_run", len),
            &len,
            |b, len| {
                let engine = typeflow_engine_new_embedded();
                assert!(!engine.is_null());
                b.iter(|| feed_letter_run(engine, *len));
                unsafe {
                    typeflow_engine_free(engine);
                }
            },
        );
    }

    group.finish();
}

fn feed_process_batch(engine: *mut typeflow_core::Engine, tokens: &[&[u8]]) {
    let mut action = blank_action();
    for token in tokens {
        unsafe {
            typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
        }
        for physical in *token {
            unsafe {
                typeflow_engine_process(engine, letter(*physical), &mut action);
            }
            black_box(action.tag);
            black_box(action.replace_text_len);
        }
    }
}

fn feed_and_apply_token(engine: *mut typeflow_core::Engine, token: &[u8]) {
    let mut action = blank_action();
    let mut committed = String::new();
    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
    }
    for physical in token {
        unsafe {
            typeflow_engine_process(engine, letter(*physical), &mut action);
        }
        apply_action(&action, &mut committed);
        black_box(&committed);
    }
}

fn feed_letter_run(engine: *mut typeflow_core::Engine, len: usize) {
    let mut action = blank_action();
    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
    }
    for _ in 0..len {
        unsafe {
            typeflow_engine_process(engine, letter(0), &mut action);
        }
        black_box(action.tag);
    }
}

const fn letter(physical: u8) -> TfEvent {
    TfEvent {
        tag: TF_EVENT_LETTER,
        physical,
        modifiers: 0,
        codepoint: 0,
    }
}

fn blank_action() -> TfAction {
    TfAction {
        tag: TF_ACTION_KEEP,
        commit_codepoint: 0,
        replace_old_len: 0,
        replace_text_len: 0,
        replace_layout: TF_LAYOUT_ENGLISH,
        replace_text: [0; TF_REPLACE_BUF_LEN],
    }
}

fn apply_action(action: &TfAction, committed: &mut String) {
    match action.tag {
        TF_ACTION_KEEP | TF_ACTION_RESET => {}
        TF_ACTION_COMMIT => {
            let character = char::from_u32(action.commit_codepoint)
                .expect("FFI commit codepoint should be a valid Unicode scalar");
            committed.push(character);
        }
        TF_ACTION_REPLACE => {
            for _ in 0..action.replace_old_len {
                committed.pop();
            }
            let replacement = std::str::from_utf8(&action.replace_text[..action.replace_text_len])
                .expect("FFI replacement should be valid UTF-8");
            committed.push_str(replacement);
        }
        _ => unreachable!("unexpected FFI action tag"),
    }
}

fn batch_key_count(tokens: &[&[u8]]) -> usize {
    tokens.iter().map(|token| token.len()).sum()
}

criterion_group!(benches, bench_ffi);
criterion_main!(benches);
