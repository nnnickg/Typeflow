#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::ffi::CStr;

use typeclaw_ffi::{
    TC_CONTEXT_AUTOMATIC_SWITCHING_DISABLED, TC_EVENT_BACKSPACE, TC_EVENT_END_TOKEN,
    TC_EVENT_LETTER, TC_EVENT_LITERAL, TC_LAYOUT_ENGLISH, TC_LAYOUT_SECONDARY, TC_OBSERVATION_NONE,
    TC_OBSERVATION_RESET_TOKEN, TC_OBSERVATION_SWITCH_FUTURE_LAYOUT, TcEngine, TcEngineConfig,
    TcEvent, TcObservation, typeclaw_engine_current_layout, typeclaw_engine_default_config,
    typeclaw_engine_force_switch_layout, typeclaw_engine_free, typeclaw_engine_new_embedded,
    typeclaw_engine_new_embedded_with_config, typeclaw_engine_observe,
    typeclaw_engine_pending_replacement_delete_count, typeclaw_engine_pending_replacement_utf8_len,
    typeclaw_engine_reset_layout, typeclaw_engine_set_host_context,
    typeclaw_engine_take_pending_replacement_utf8,
};

fn blank_observation() -> TcObservation {
    TcObservation {
        tag: TC_OBSERVATION_NONE,
        layout: TC_LAYOUT_ENGLISH,
    }
}

fn letter(physical: u8) -> TcEvent {
    TcEvent {
        tag: TC_EVENT_LETTER,
        physical,
        modifiers: 0,
        codepoint: 0,
    }
}

fn literal(character: char) -> TcEvent {
    TcEvent {
        tag: TC_EVENT_LITERAL,
        physical: 0,
        modifiers: 0,
        codepoint: character as u32,
    }
}

fn backspace() -> TcEvent {
    TcEvent {
        tag: TC_EVENT_BACKSPACE,
        physical: 0,
        modifiers: 0,
        codepoint: 0,
    }
}

fn end_token() -> TcEvent {
    TcEvent {
        tag: TC_EVENT_END_TOKEN,
        physical: 0,
        modifiers: 0,
        codepoint: 0,
    }
}

fn observe(engine: *mut TcEngine, event: TcEvent) -> TcObservation {
    let mut observation = blank_observation();
    unsafe {
        typeclaw_engine_observe(engine, event, &mut observation);
    }
    observation
}

fn take_pending_replacement_text(engine: *mut TcEngine) -> Option<String> {
    let len = unsafe { typeclaw_engine_pending_replacement_utf8_len(engine) };
    if len == 0 {
        return None;
    }

    let mut buffer = vec![0i8; len + 1];
    let written = unsafe {
        typeclaw_engine_take_pending_replacement_utf8(engine, buffer.as_mut_ptr(), buffer.len())
    };
    assert_eq!(written, len);
    Some(
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[test]
fn public_abi_observes_then_resets_wrong_layout_token() {
    let engine = typeclaw_engine_new_embedded();
    assert!(!engine.is_null());

    let mut observation = observe(engine, letter(6));
    assert_eq!(observation.tag, TC_OBSERVATION_NONE);
    let mut saw_secondary_switch = false;
    for physical in [7, 18, 3, 1, 13] {
        observation = observe(engine, letter(physical));
        saw_secondary_switch |= observation.tag == TC_OBSERVATION_SWITCH_FUTURE_LAYOUT
            && observation.layout == TC_LAYOUT_SECONDARY;
    }

    assert!(saw_secondary_switch);
    assert_eq!(
        unsafe { typeclaw_engine_current_layout(engine) },
        TC_LAYOUT_SECONDARY
    );

    observation = observe(engine, end_token());
    assert_eq!(observation.tag, TC_OBSERVATION_RESET_TOKEN);

    unsafe {
        typeclaw_engine_free(engine);
    }
}

#[test]
fn public_abi_literals_and_backspace_stay_in_sync() {
    let engine = typeclaw_engine_new_embedded();
    assert!(!engine.is_null());

    for physical in [0, 1, 2] {
        let observation = observe(engine, letter(physical));
        assert_eq!(observation.tag, TC_OBSERVATION_NONE);
    }

    let observation = observe(engine, literal('1'));
    assert_eq!(observation.tag, TC_OBSERVATION_RESET_TOKEN);

    for physical in [3, 4, 5] {
        let observation = observe(engine, letter(physical));
        assert_eq!(observation.tag, TC_OBSERVATION_NONE);
    }

    let observation = observe(engine, backspace());
    assert_eq!(observation.tag, TC_OBSERVATION_NONE);

    observe(engine, backspace());
    let observation = observe(engine, backspace());
    assert_eq!(observation.tag, TC_OBSERVATION_RESET_TOKEN);
    assert_eq!(
        unsafe { typeclaw_engine_current_layout(engine) },
        TC_LAYOUT_ENGLISH
    );

    unsafe {
        typeclaw_engine_free(engine);
    }
}

#[test]
fn public_abi_auto_switching_disabled_observes_without_switching() {
    let engine = typeclaw_engine_new_embedded();
    assert!(!engine.is_null());

    unsafe {
        typeclaw_engine_set_host_context(engine, TC_CONTEXT_AUTOMATIC_SWITCHING_DISABLED);
    }

    for physical in [6, 7, 18, 3, 1, 13] {
        let observation = observe(engine, letter(physical));
        assert_eq!(observation.tag, TC_OBSERVATION_NONE);
    }
    let observation = observe(engine, end_token());
    assert_eq!(observation.tag, TC_OBSERVATION_RESET_TOKEN);
    assert_eq!(
        unsafe { typeclaw_engine_current_layout(engine) },
        TC_LAYOUT_ENGLISH
    );

    unsafe {
        typeclaw_engine_reset_layout(engine, TC_LAYOUT_SECONDARY);
    }
    for physical in [6, 7, 18, 3, 1, 13] {
        let observation = observe(engine, letter(physical));
        assert_eq!(observation.tag, TC_OBSERVATION_NONE);
    }
    let observation = observe(engine, end_token());
    assert_eq!(observation.tag, TC_OBSERVATION_RESET_TOKEN);
    assert_eq!(
        unsafe { typeclaw_engine_current_layout(engine) },
        TC_LAYOUT_SECONDARY
    );

    unsafe {
        typeclaw_engine_free(engine);
    }
}

#[test]
fn public_abi_force_switch_changes_future_layout_only() {
    let engine = typeclaw_engine_new_embedded();
    assert!(!engine.is_null());

    for physical in [19, 24, 15, 4] {
        observe(engine, letter(physical));
    }

    let mut observation = blank_observation();
    unsafe {
        typeclaw_engine_force_switch_layout(engine, &mut observation);
    }
    assert_eq!(observation.tag, TC_OBSERVATION_SWITCH_FUTURE_LAYOUT);
    assert_eq!(observation.layout, TC_LAYOUT_SECONDARY);
    assert_eq!(
        unsafe { typeclaw_engine_pending_replacement_delete_count(engine) },
        4
    );
    assert_eq!(
        take_pending_replacement_text(engine).as_deref(),
        Some("ензу")
    );
    assert_eq!(
        unsafe { typeclaw_engine_pending_replacement_delete_count(engine) },
        0
    );

    unsafe {
        typeclaw_engine_free(engine);
    }
}

#[test]
fn public_abi_accepts_explicit_config() {
    let mut config = TcEngineConfig {
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
        typeclaw_engine_default_config(&mut config);
    }
    assert_eq!(config.min_token_len, 4);
    assert_eq!(config.max_token_len, 128);

    config.min_token_len = 6;
    let engine = typeclaw_engine_new_embedded_with_config(config);
    assert!(!engine.is_null());

    unsafe {
        typeclaw_engine_free(engine);
    }
}
