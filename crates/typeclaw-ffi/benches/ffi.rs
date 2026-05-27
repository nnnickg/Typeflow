#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::ffi::CString;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use typeclaw_ffi::{
    TC_EVENT_LETTER, TC_HOST_POLICY_REASON_NORMAL, TC_LAYOUT_ENGLISH, TC_OBSERVATION_NONE,
    TcEngine, TcEvent, TcHostInputPolicy, TcHostSurfaceFacts, TcObservation, typeclaw_engine_free,
    typeclaw_engine_new_embedded, typeclaw_engine_observe, typeclaw_engine_reset_layout,
    typeclaw_host_config_free, typeclaw_host_config_load_defaults,
    typeclaw_host_config_resolve_input_policy,
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
        let engine = typeclaw_engine_new_embedded();
        assert!(!engine.is_null());
        b.iter(|| feed_process_batch(engine, MIXED_TOKENS));
        unsafe {
            typeclaw_engine_free(engine);
        }
    });

    group.throughput(Throughput::Elements(batch_key_count(MIXED_TOKENS) as u64));
    group.bench_function("observe_mixed_physical_batch_new_output_each_key", |b| {
        let engine = typeclaw_engine_new_embedded();
        assert!(!engine.is_null());
        b.iter(|| feed_process_batch_new_action_each_key(engine, MIXED_TOKENS));
        unsafe {
            typeclaw_engine_free(engine);
        }
    });

    group.bench_function("new_embedded_engine_cached", |b| {
        b.iter(|| {
            let engine = typeclaw_engine_new_embedded();
            assert!(!engine.is_null());
            black_box(engine);
            unsafe {
                typeclaw_engine_free(engine);
            }
        });
    });

    group.bench_function("resolve_host_policy_cached_facts", |b| {
        let config = typeclaw_host_config_load_defaults();
        assert!(!config.is_null());
        let bundle = CString::new("dev.zed.Zed").unwrap();
        let client = CString::new("CGEventTap").unwrap();
        let identifier = CString::new("source-editor").unwrap();
        let facts = TcHostSurfaceFacts {
            secure_input: 0,
            bundle_id_utf8: bundle.as_ptr(),
            application_name_utf8: std::ptr::null(),
            input_client_class_utf8: client.as_ptr(),
            focused_element_role_utf8: std::ptr::null(),
            focused_element_subrole_utf8: std::ptr::null(),
            focused_element_role_description_utf8: std::ptr::null(),
            focused_element_identifier_utf8: identifier.as_ptr(),
            focused_element_description_utf8: std::ptr::null(),
            focused_element_context_utf8: std::ptr::null(),
            focused_window_title_utf8: std::ptr::null(),
        };
        let mut policy = TcHostInputPolicy {
            flags: 0,
            reason: TC_HOST_POLICY_REASON_NORMAL,
        };
        b.iter(|| unsafe {
            typeclaw_host_config_resolve_input_policy(config, facts, &mut policy);
            black_box(policy);
        });
        unsafe {
            typeclaw_host_config_free(config);
        }
    });

    group.throughput(Throughput::Elements(WRONG_LAYOUT_TOKEN.len() as u64));
    group.bench_function("observe_wrong_layout_token", |b| {
        let engine = typeclaw_engine_new_embedded();
        assert!(!engine.is_null());
        b.iter(|| feed_token(engine, WRONG_LAYOUT_TOKEN));
        unsafe {
            typeclaw_engine_free(engine);
        }
    });

    for len in [64usize, 256, 1024] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_with_input(
            BenchmarkId::new("process_letter_run", len),
            &len,
            |b, len| {
                let engine = typeclaw_engine_new_embedded();
                assert!(!engine.is_null());
                b.iter(|| feed_letter_run(engine, *len));
                unsafe {
                    typeclaw_engine_free(engine);
                }
            },
        );
    }

    group.finish();
}

fn feed_process_batch(engine: *mut TcEngine, tokens: &[&[u8]]) {
    let mut observation = blank_observation();
    for token in tokens {
        unsafe {
            typeclaw_engine_reset_layout(engine, TC_LAYOUT_ENGLISH);
        }
        for physical in *token {
            unsafe {
                typeclaw_engine_observe(engine, letter(*physical), &mut observation);
            }
            black_box(observation.tag);
            black_box(observation.layout);
        }
    }
}

fn feed_process_batch_new_action_each_key(engine: *mut TcEngine, tokens: &[&[u8]]) {
    for token in tokens {
        unsafe {
            typeclaw_engine_reset_layout(engine, TC_LAYOUT_ENGLISH);
        }
        for physical in *token {
            let mut observation = blank_observation();
            unsafe {
                typeclaw_engine_observe(engine, letter(*physical), &mut observation);
            }
            black_box(observation.tag);
            black_box(observation.layout);
        }
    }
}

fn feed_token(engine: *mut TcEngine, token: &[u8]) {
    let mut observation = blank_observation();
    unsafe {
        typeclaw_engine_reset_layout(engine, TC_LAYOUT_ENGLISH);
    }
    for physical in token {
        unsafe {
            typeclaw_engine_observe(engine, letter(*physical), &mut observation);
        }
        black_box(observation.tag);
        black_box(observation.layout);
    }
}

fn feed_letter_run(engine: *mut TcEngine, len: usize) {
    let mut observation = blank_observation();
    unsafe {
        typeclaw_engine_reset_layout(engine, TC_LAYOUT_ENGLISH);
    }
    for _ in 0..len {
        unsafe {
            typeclaw_engine_observe(engine, letter(0), &mut observation);
        }
        black_box(observation.tag);
    }
}

const fn letter(physical: u8) -> TcEvent {
    TcEvent {
        tag: TC_EVENT_LETTER,
        physical,
        modifiers: 0,
        codepoint: 0,
    }
}

fn blank_observation() -> TcObservation {
    TcObservation {
        tag: TC_OBSERVATION_NONE,
        layout: TC_LAYOUT_ENGLISH,
    }
}

fn batch_key_count(tokens: &[&[u8]]) -> usize {
    tokens.iter().map(|token| token.len()).sum()
}

criterion_group!(benches, bench_ffi);
criterion_main!(benches);
