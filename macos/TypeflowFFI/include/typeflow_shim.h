#ifndef TYPEFLOW_FFI_SHIM_H
#define TYPEFLOW_FFI_SHIM_H

#include <string.h>

#include "../../../crates/typeflow-ffi/include/typeflow.h"

static inline TfEvent typeflow_ffi_letter_event(uint8_t physical, uint8_t modifiers) {
    TfEvent event;
    event.tag = TF_EVENT_LETTER;
    event.physical = physical;
    event.modifiers = modifiers;
    event.codepoint = 0;
    return event;
}

static inline TfEvent typeflow_ffi_end_token_event(void) {
    TfEvent event;
    event.tag = TF_EVENT_END_TOKEN;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = 0;
    return event;
}

static inline TfEvent typeflow_ffi_backspace_event(void) {
    TfEvent event;
    event.tag = TF_EVENT_BACKSPACE;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = 0;
    return event;
}

static inline TfEvent typeflow_ffi_literal_event(uint32_t codepoint) {
    TfEvent event;
    event.tag = TF_EVENT_LITERAL;
    event.physical = 0;
    event.modifiers = 0;
    event.codepoint = codepoint;
    return event;
}

static inline TfEvent typeflow_ffi_host_bypass_event(uint8_t modifiers) {
    TfEvent event;
    event.tag = TF_EVENT_LETTER;
    event.physical = 0;
    event.modifiers = modifiers;
    event.codepoint = 0;
    return event;
}

static inline TfAction typeflow_ffi_empty_action(void) {
    TfAction action;
    memset(&action, 0, sizeof(action));
    return action;
}

#endif
