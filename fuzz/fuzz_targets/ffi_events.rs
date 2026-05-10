#![no_main]

use std::ffi::CString;

use libfuzzer_sys::fuzz_target;
use typeflow_ffi::{
    TF_LAYOUT_ENGLISH, TF_LAYOUT_SECONDARY, TF_REPLACE_BUF_LEN, TfAction, TfEvent,
    typeflow_engine_convert_visible_tail, typeflow_engine_convert_visible_token,
    typeflow_engine_force_switch_token, typeflow_engine_free, typeflow_engine_new_embedded,
    typeflow_engine_process, typeflow_engine_replace_visible_prefix_with_key,
    typeflow_engine_replace_visible_tail_with_key,
};

const MAX_EVENTS_PER_INPUT: usize = 256;
const MAX_TEXT_BYTES: usize = 256;

fuzz_target!(|data: &[u8]| {
    let engine = typeflow_engine_new_embedded();
    if engine.is_null() {
        return;
    }

    let mut action = empty_action();
    for chunk in data.chunks(8).take(MAX_EVENTS_PER_INPUT) {
        let event = TfEvent {
            tag: byte(chunk, 0),
            physical: byte(chunk, 1),
            modifiers: byte(chunk, 2),
            codepoint: u32::from_le_bytes([
                byte(chunk, 3),
                byte(chunk, 4),
                byte(chunk, 5),
                byte(chunk, 6),
            ]),
        };
        unsafe {
            typeflow_engine_process(engine, event, &mut action);
        }

        if byte(chunk, 7) & 0x01 != 0 {
            unsafe {
                typeflow_engine_force_switch_token(engine, &mut action);
            }
        }
    }

    if let Some(text) = fuzz_text(data) {
        let physical = byte(data, 0);
        let modifiers = byte(data, 1);
        let layout = if byte(data, 2) & 0x01 == 0 {
            TF_LAYOUT_ENGLISH
        } else {
            TF_LAYOUT_SECONDARY
        };

        unsafe {
            typeflow_engine_convert_visible_token(engine, text.as_ptr(), &mut action);
            typeflow_engine_convert_visible_tail(engine, text.as_ptr(), &mut action);
            typeflow_engine_replace_visible_prefix_with_key(
                engine,
                text.as_ptr(),
                physical,
                modifiers,
                layout,
                &mut action,
            );
            typeflow_engine_replace_visible_tail_with_key(
                engine,
                text.as_ptr(),
                physical,
                modifiers,
                layout,
                &mut action,
            );
        }
    }

    unsafe {
        typeflow_engine_free(engine);
    }
});

fn byte(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or_default()
}

fn empty_action() -> TfAction {
    TfAction {
        tag: 0,
        commit_codepoint: 0,
        replace_old_len: 0,
        replace_text_len: 0,
        replace_layout: 0,
        replace_text: [0; TF_REPLACE_BUF_LEN],
    }
}

fn fuzz_text(data: &[u8]) -> Option<CString> {
    let bytes = data
        .iter()
        .copied()
        .take(MAX_TEXT_BYTES)
        .map(|byte| if byte == 0 { b' ' } else { byte })
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&bytes);
    CString::new(text.as_bytes()).ok()
}
