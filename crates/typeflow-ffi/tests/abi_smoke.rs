#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use typeflow_ffi::{
    TF_COMPOSITION_BYPASS, TF_COMPOSITION_CLEAR, TF_COMPOSITION_COMMIT, TF_COMPOSITION_RENDER,
    TF_COMPOSITION_TEXT_BUF_LEN, TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED, TF_EVENT_BACKSPACE,
    TF_EVENT_END_TOKEN, TF_EVENT_LETTER, TF_EVENT_LITERAL, TF_LAYOUT_ENGLISH, TF_LAYOUT_SECONDARY,
    TfComposition, TfEngine, TfEngineConfig, TfEvent, typeflow_engine_current_layout,
    typeflow_engine_default_config, typeflow_engine_force_switch_token, typeflow_engine_free,
    typeflow_engine_new_embedded, typeflow_engine_new_embedded_with_config,
    typeflow_engine_process, typeflow_engine_reset_layout, typeflow_engine_set_host_context,
};

fn blank_composition() -> TfComposition {
    TfComposition {
        tag: TF_COMPOSITION_BYPASS,
        consume_event: 0,
        layout: TF_LAYOUT_ENGLISH,
        text_len: 0,
        text: [0; TF_COMPOSITION_TEXT_BUF_LEN],
    }
}

fn letter(physical: u8) -> TfEvent {
    TfEvent {
        tag: TF_EVENT_LETTER,
        physical,
        modifiers: 0,
        codepoint: 0,
    }
}

fn literal(character: char) -> TfEvent {
    TfEvent {
        tag: TF_EVENT_LITERAL,
        physical: 0,
        modifiers: 0,
        codepoint: character as u32,
    }
}

fn backspace() -> TfEvent {
    TfEvent {
        tag: TF_EVENT_BACKSPACE,
        physical: 0,
        modifiers: 0,
        codepoint: 0,
    }
}

fn end_token() -> TfEvent {
    TfEvent {
        tag: TF_EVENT_END_TOKEN,
        physical: 0,
        modifiers: 0,
        codepoint: 0,
    }
}

fn process(engine: *mut TfEngine, event: TfEvent) -> TfComposition {
    let mut composition = blank_composition();
    unsafe {
        typeflow_engine_process(engine, event, &mut composition);
    }
    composition
}

fn composition_text(composition: &TfComposition) -> &str {
    std::str::from_utf8(&composition.text[..composition.text_len])
        .expect("FFI composition text should be valid UTF-8")
}

fn apply_composition(
    composition: &TfComposition,
    fallback: Option<char>,
    committed: &mut String,
    composing: &mut String,
) {
    match composition.tag {
        TF_COMPOSITION_BYPASS => {
            if let Some(character) = fallback {
                committed.push(character);
            }
        }
        TF_COMPOSITION_RENDER => {
            composing.clear();
            composing.push_str(composition_text(composition));
        }
        TF_COMPOSITION_COMMIT => {
            committed.push_str(composition_text(composition));
            composing.clear();
            if composition.consume_event == 0
                && let Some(character) = fallback
            {
                committed.push(character);
            }
        }
        TF_COMPOSITION_CLEAR => {
            composing.clear();
            if composition.consume_event == 0
                && let Some(character) = fallback
            {
                committed.push(character);
            }
        }
        other => panic!("unexpected FFI composition tag {other}"),
    }
}

#[test]
fn public_abi_renders_then_commits_wrong_layout_token() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    let mut committed = String::new();
    let mut composing = String::new();
    let mut composition = process(engine, letter(6));
    assert_eq!(composition.tag, TF_COMPOSITION_RENDER);
    apply_composition(&composition, None, &mut committed, &mut composing);
    for physical in [7, 18, 3, 1, 13] {
        composition = process(engine, letter(physical));
        assert_eq!(composition.tag, TF_COMPOSITION_RENDER);
        apply_composition(&composition, None, &mut committed, &mut composing);
    }

    assert_eq!(committed, "");
    assert_eq!(composing, "привіт");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_SECONDARY
    );

    composition = process(engine, end_token());
    assert_eq!(composition.tag, TF_COMPOSITION_COMMIT);
    assert_eq!(composition.consume_event, 0);
    apply_composition(&composition, None, &mut committed, &mut composing);
    assert_eq!(committed, "привіт");
    assert_eq!(composing, "");

    unsafe {
        typeflow_engine_free(engine);
    }
}

#[test]
fn public_abi_literals_and_backspace_stay_in_sync() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    let mut committed = String::new();
    let mut composing = String::new();
    for physical in [0, 1, 2] {
        let composition = process(engine, letter(physical));
        apply_composition(&composition, None, &mut committed, &mut composing);
    }

    let composition = process(engine, literal('1'));
    assert_eq!(composition.tag, TF_COMPOSITION_COMMIT);
    assert_eq!(composition_text(&composition), "abc1");
    apply_composition(&composition, Some('1'), &mut committed, &mut composing);

    for physical in [3, 4, 5] {
        let composition = process(engine, letter(physical));
        apply_composition(&composition, None, &mut committed, &mut composing);
    }
    assert_eq!(committed, "abc1");
    assert_eq!(composing, "def");

    let composition = process(engine, backspace());
    assert_eq!(composition.tag, TF_COMPOSITION_RENDER);
    assert_eq!(composition_text(&composition), "de");
    apply_composition(&composition, None, &mut committed, &mut composing);

    process(engine, backspace());
    let composition = process(engine, backspace());
    assert_eq!(composition.tag, TF_COMPOSITION_CLEAR);
    assert_eq!(composition.consume_event, 1);
    apply_composition(&composition, None, &mut committed, &mut composing);

    assert_eq!(committed, "abc1");
    assert_eq!(composing, "");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_ENGLISH
    );

    unsafe {
        typeflow_engine_free(engine);
    }
}

#[test]
fn public_abi_auto_switching_disabled_renders_current_layout() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    unsafe {
        typeflow_engine_set_host_context(engine, TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED);
    }

    let mut committed = String::new();
    let mut composing = String::new();
    for physical in [6, 7, 18, 3, 1, 13] {
        let composition = process(engine, letter(physical));
        apply_composition(&composition, None, &mut committed, &mut composing);
    }
    let composition = process(engine, end_token());
    apply_composition(&composition, None, &mut committed, &mut composing);
    assert_eq!(committed, "ghsdbn");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_ENGLISH
    );

    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_SECONDARY);
    }
    committed.clear();
    composing.clear();
    for physical in [6, 7, 18, 3, 1, 13] {
        let composition = process(engine, letter(physical));
        apply_composition(&composition, None, &mut committed, &mut composing);
    }
    let composition = process(engine, end_token());
    apply_composition(&composition, None, &mut committed, &mut composing);
    assert_eq!(committed, "привіт");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_SECONDARY
    );

    unsafe {
        typeflow_engine_free(engine);
    }
}

#[test]
fn public_abi_force_switch_rerenders_active_composition() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    for physical in [19, 24, 15, 4] {
        process(engine, letter(physical));
    }

    let mut composition = blank_composition();
    unsafe {
        typeflow_engine_force_switch_token(engine, &mut composition);
    }
    assert_eq!(composition.tag, TF_COMPOSITION_RENDER);
    assert_eq!(composition.layout, TF_LAYOUT_SECONDARY);
    assert_eq!(composition_text(&composition), "ензу");

    unsafe {
        typeflow_engine_free(engine);
    }
}

#[test]
fn public_abi_accepts_explicit_config() {
    let mut config = TfEngineConfig {
        min_token_len: 0,
        max_token_len: 0,
        confidence_margin: 0.0,
        dict_exact_weight: 0.0,
        dict_prefix_weight: 0.0,
        ngram_only_confidence_margin: 0.0,
        bigram_weight: 0.0,
        trigram_weight: 0.0,
        length_normalize: 0,
        disable_on_internal_caps: 0,
    };

    unsafe {
        typeflow_engine_default_config(&mut config);
    }
    assert_eq!(config.min_token_len, 4);
    assert_eq!(config.max_token_len, 128);

    config.min_token_len = 6;
    let engine = typeflow_engine_new_embedded_with_config(config);
    assert!(!engine.is_null());

    unsafe {
        typeflow_engine_free(engine);
    }
}
