#ifndef TYPECLAW_FFI_SHIM_H
#define TYPECLAW_FFI_SHIM_H

#include <string.h>

#include "../../../crates/typeclaw-ffi/include/typeclaw.h"

static inline uint32_t typeclaw_ffi_context_secure_input(void) {
    return TC_CONTEXT_SECURE_INPUT;
}

static inline uint32_t typeclaw_ffi_context_automatic_processing_disabled(void) {
    return TC_CONTEXT_AUTOMATIC_PROCESSING_DISABLED;
}

static inline uint32_t typeclaw_ffi_context_automatic_switching_disabled(void) {
    return TC_CONTEXT_AUTOMATIC_SWITCHING_DISABLED;
}

static inline TcEvent typeclaw_ffi_letter_event(uint8_t physical, uint8_t modifiers) {
    TcEvent event;
    event.tag = TC_EVENT_LETTER;
    event.physical = physical;
    event.modifiers = modifiers;
    event.codepoint = 0;
    return event;
}

static inline TcEvent typeclaw_ffi_end_token_event(void) {
    TcEvent event;
    event.tag = TC_EVENT_END_TOKEN;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = 0;
    return event;
}

static inline TcEvent typeclaw_ffi_backspace_event(void) {
    TcEvent event;
    event.tag = TC_EVENT_BACKSPACE;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = 0;
    return event;
}

static inline TcEvent typeclaw_ffi_literal_event(uint32_t codepoint) {
    TcEvent event;
    event.tag = TC_EVENT_LITERAL;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = codepoint;
    return event;
}

static inline TcEvent typeclaw_ffi_host_bypass_event(uint8_t modifiers) {
    TcEvent event;
    event.tag = TC_EVENT_LETTER;
    event.physical = 0;
    event.modifiers = modifiers;
    event.codepoint = 0;
    return event;
}

static inline TcObservation typeclaw_ffi_empty_observation(void) {
    TcObservation observation;
    memset(&observation, 0, sizeof(observation));
    return observation;
}

#endif
