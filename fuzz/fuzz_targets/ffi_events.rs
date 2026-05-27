#![no_main]

use libfuzzer_sys::fuzz_target;
use typeclaw_ffi::{
    TC_LAYOUT_ENGLISH, TC_OBSERVATION_NONE, TcEvent, TcObservation,
    typeclaw_engine_force_switch_layout, typeclaw_engine_free, typeclaw_engine_new_embedded,
    typeclaw_engine_observe,
};

const MAX_EVENTS_PER_INPUT: usize = 256;

fuzz_target!(|data: &[u8]| {
    let engine = typeclaw_engine_new_embedded();
    if engine.is_null() {
        return;
    }

    let mut observation = empty_observation();
    for chunk in data.chunks(8).take(MAX_EVENTS_PER_INPUT) {
        let event = TcEvent {
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
            typeclaw_engine_observe(engine, event, &mut observation);
        }

        if byte(chunk, 7) & 0x01 != 0 {
            unsafe {
                typeclaw_engine_force_switch_layout(engine, &mut observation);
            }
        }
    }

    unsafe {
        typeclaw_engine_free(engine);
    }
});

fn byte(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or_default()
}

fn empty_observation() -> TcObservation {
    TcObservation {
        tag: TC_OBSERVATION_NONE,
        layout: TC_LAYOUT_ENGLISH,
    }
}
