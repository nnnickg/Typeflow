use std::ffi::CString;

use typeflow_ffi::{
    TF_ACTION_COMMIT, TF_ACTION_KEEP, TF_ACTION_REPLACE, TF_ACTION_RESET,
    TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED, TF_EVENT_BACKSPACE, TF_EVENT_LETTER, TF_EVENT_LITERAL,
    TF_LAYOUT_ENGLISH, TF_LAYOUT_SECONDARY, TF_REPLACE_BUF_LEN, TfAction, TfEngineConfig, TfEvent,
    typeflow_engine_convert_visible_tail, typeflow_engine_current_layout,
    typeflow_engine_default_config, typeflow_engine_free, typeflow_engine_new_embedded,
    typeflow_engine_new_embedded_with_config, typeflow_engine_process,
    typeflow_engine_replace_visible_tail_with_key, typeflow_engine_reset_layout,
    typeflow_engine_set_host_context,
};

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

fn process(engine: *mut typeflow_core::Engine, event: TfEvent) -> TfAction {
    let mut action = blank_action();
    unsafe {
        typeflow_engine_process(engine, event, &mut action);
    }
    action
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
        other => panic!("unexpected FFI action tag {other}"),
    }
}

fn replacement_text(action: &TfAction) -> &str {
    std::str::from_utf8(&action.replace_text[..action.replace_text_len])
        .expect("FFI replacement should be valid UTF-8")
}

#[test]
fn public_abi_replaces_wrong_layout_token() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    let mut committed = String::new();
    for physical in [6, 7, 18, 3, 1, 13] {
        let action = process(engine, letter(physical));
        apply_action(&action, &mut committed);
    }

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
fn public_abi_literals_and_backspace_stay_in_sync() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    let mut committed = String::new();
    for physical in [0, 1, 2] {
        let action = process(engine, letter(physical));
        apply_action(&action, &mut committed);
    }

    let action = process(engine, literal('1'));
    apply_action(&action, &mut committed);

    for physical in [3, 4, 5] {
        let action = process(engine, letter(physical));
        apply_action(&action, &mut committed);
    }

    assert_eq!(committed, "abc1def");

    for _ in 0..3 {
        let action = process(engine, backspace());
        assert_eq!(action.tag, TF_ACTION_KEEP);
        committed.pop();
    }

    assert_eq!(committed, "abc1");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_ENGLISH
    );

    unsafe {
        typeflow_engine_free(engine);
    }
}

#[test]
fn public_abi_auto_switching_disabled_commits_current_layout() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    unsafe {
        typeflow_engine_set_host_context(engine, TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED);
    }

    let mut committed = String::new();
    for physical in [6, 7, 18, 3, 1, 13] {
        let action = process(engine, letter(physical));
        apply_action(&action, &mut committed);
    }
    assert_eq!(committed, "ghsdbn");
    assert_eq!(
        unsafe { typeflow_engine_current_layout(engine) },
        TF_LAYOUT_ENGLISH
    );

    unsafe {
        typeflow_engine_reset_layout(engine, TF_LAYOUT_SECONDARY);
    }
    committed.clear();
    for physical in [6, 7, 18, 3, 1, 13] {
        let action = process(engine, letter(physical));
        apply_action(&action, &mut committed);
    }
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

#[test]
fn public_abi_visible_tail_keeps_punctuation_position_letters() {
    let engine = typeflow_engine_new_embedded();
    assert!(!engine.is_null());

    let tail = CString::new("hello [fn").unwrap();
    let mut action = blank_action();
    unsafe {
        typeflow_engine_replace_visible_tail_with_key(
            engine,
            tail.as_ptr(),
            5,
            0,
            TF_LAYOUT_SECONDARY,
            &mut action,
        );
    }

    assert_eq!(action.tag, TF_ACTION_REPLACE);
    assert_eq!(action.replace_old_len, 3);
    assert_eq!(action.replace_layout, TF_LAYOUT_SECONDARY);
    assert_eq!(replacement_text(&action), "хата");

    let tail = CString::new("hello [fnf").unwrap();
    let mut action = blank_action();
    unsafe {
        typeflow_engine_convert_visible_tail(engine, tail.as_ptr(), &mut action);
    }

    assert_eq!(action.tag, TF_ACTION_REPLACE);
    assert_eq!(action.replace_old_len, 4);
    assert_eq!(action.replace_layout, TF_LAYOUT_SECONDARY);
    assert_eq!(replacement_text(&action), "хата");

    let tail = CString::new("hello ’dh").unwrap();
    let mut action = blank_action();
    unsafe {
        typeflow_engine_replace_visible_tail_with_key(
            engine,
            tail.as_ptr(),
            9,
            0,
            TF_LAYOUT_SECONDARY,
            &mut action,
        );
    }

    assert_eq!(action.tag, TF_ACTION_REPLACE);
    assert_eq!(action.replace_old_len, 3);
    assert_eq!(action.replace_layout, TF_LAYOUT_SECONDARY);
    assert_eq!(replacement_text(&action), "євро");

    let tail = CString::new("hello ’dhj").unwrap();
    let mut action = blank_action();
    unsafe {
        typeflow_engine_convert_visible_tail(engine, tail.as_ptr(), &mut action);
    }

    assert_eq!(action.tag, TF_ACTION_REPLACE);
    assert_eq!(action.replace_old_len, 4);
    assert_eq!(action.replace_layout, TF_LAYOUT_SECONDARY);
    assert_eq!(replacement_text(&action), "євро");

    unsafe {
        typeflow_engine_free(engine);
    }
}
