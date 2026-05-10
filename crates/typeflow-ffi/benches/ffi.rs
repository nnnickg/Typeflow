#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::ffi::CString;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use typeflow_ffi::{
    TF_COMPOSITION_BYPASS, TF_COMPOSITION_CLEAR, TF_COMPOSITION_COMMIT, TF_COMPOSITION_RENDER,
    TF_COMPOSITION_TEXT_BUF_LEN, TF_EVENT_LETTER, TF_HOST_POLICY_REASON_NORMAL, TF_LAYOUT_ENGLISH,
    TfComposition, TfEngine, TfEvent, TfHostInputPolicy, TfHostSurfaceFacts, typeflow_engine_free,
    typeflow_engine_new_embedded, typeflow_engine_process, typeflow_engine_reset_layout,
    typeflow_host_config_free, typeflow_host_config_load_defaults,
    typeflow_host_config_resolve_input_policy,
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

    group.throughput(Throughput::Elements(batch_key_count(MIXED_TOKENS) as u64));
    group.bench_function(
        "process_mixed_physical_batch_new_composition_each_key",
        |b| {
            let engine = typeflow_engine_new_embedded();
            assert!(!engine.is_null());
            b.iter(|| feed_process_batch_new_action_each_key(engine, MIXED_TOKENS));
            unsafe {
                typeflow_engine_free(engine);
            }
        },
    );

    group.bench_function("new_embedded_engine_cached", |b| {
        b.iter(|| {
            let engine = typeflow_engine_new_embedded();
            assert!(!engine.is_null());
            black_box(engine);
            unsafe {
                typeflow_engine_free(engine);
            }
        });
    });

    group.bench_function("resolve_host_policy_cached_facts", |b| {
        let config = typeflow_host_config_load_defaults();
        assert!(!config.is_null());
        let bundle = CString::new("dev.zed.Zed").unwrap();
        let client = CString::new("IMKTextInput").unwrap();
        let identifier = CString::new("source-editor").unwrap();
        let facts = TfHostSurfaceFacts {
            secure_input: 0,
            bundle_id_utf8: bundle.as_ptr(),
            application_name_utf8: std::ptr::null(),
            input_client_class_utf8: client.as_ptr(),
            focused_element_role_utf8: std::ptr::null(),
            focused_element_subrole_utf8: std::ptr::null(),
            focused_element_role_description_utf8: std::ptr::null(),
            focused_element_identifier_utf8: identifier.as_ptr(),
            focused_element_description_utf8: std::ptr::null(),
            focused_window_title_utf8: std::ptr::null(),
        };
        let mut policy = TfHostInputPolicy {
            flags: 0,
            reason: TF_HOST_POLICY_REASON_NORMAL,
        };
        b.iter(|| unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
            black_box(policy);
        });
        unsafe {
            typeflow_host_config_free(config);
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

fn feed_process_batch(engine: *mut TfEngine, tokens: &[&[u8]]) {
    let mut composition = blank_composition();
    for token in tokens {
        unsafe {
            typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
        }
        for physical in *token {
            unsafe {
                typeflow_engine_process(engine, letter(*physical), &mut composition);
            }
            black_box(composition.tag);
            black_box(composition.text_len);
        }
    }
}

fn feed_process_batch_new_action_each_key(engine: *mut TfEngine, tokens: &[&[u8]]) {
    for token in tokens {
        unsafe {
            typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
        }
        for physical in *token {
            let mut composition = blank_composition();
            unsafe {
                typeflow_engine_process(engine, letter(*physical), &mut composition);
            }
            black_box(composition.tag);
            black_box(composition.text_len);
        }
    }
}

fn feed_and_apply_token(engine: *mut TfEngine, token: &[u8]) {
    let mut composition = blank_composition();
    let mut committed = String::new();
    let mut composing = String::new();
    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
    }
    for physical in token {
        unsafe {
            typeflow_engine_process(engine, letter(*physical), &mut composition);
        }
        apply_composition(&composition, &mut committed, &mut composing);
        black_box(&committed);
        black_box(&composing);
    }
}

fn feed_letter_run(engine: *mut TfEngine, len: usize) {
    let mut composition = blank_composition();
    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_ENGLISH);
    }
    for _ in 0..len {
        unsafe {
            typeflow_engine_process(engine, letter(0), &mut composition);
        }
        black_box(composition.tag);
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

fn blank_composition() -> TfComposition {
    TfComposition {
        tag: TF_COMPOSITION_BYPASS,
        consume_event: 0,
        layout: TF_LAYOUT_ENGLISH,
        text_len: 0,
        text: [0; TF_COMPOSITION_TEXT_BUF_LEN],
    }
}

fn apply_composition(composition: &TfComposition, committed: &mut String, composing: &mut String) {
    match composition.tag {
        TF_COMPOSITION_BYPASS => {}
        TF_COMPOSITION_RENDER => {
            composing.clear();
            composing.push_str(composition_text(composition));
        }
        TF_COMPOSITION_COMMIT => {
            committed.push_str(composition_text(composition));
            composing.clear();
        }
        TF_COMPOSITION_CLEAR => {
            composing.clear();
        }
        _ => unreachable!("unexpected FFI composition tag"),
    }
}

fn composition_text(composition: &TfComposition) -> &str {
    std::str::from_utf8(&composition.text[..composition.text_len])
        .expect("FFI composition should be valid UTF-8")
}

fn batch_key_count(tokens: &[&[u8]]) -> usize {
    tokens.iter().map(|token| token.len()).sum()
}

criterion_group!(benches, bench_ffi);
criterion_main!(benches);
