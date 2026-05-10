#ifndef TYPEFLOW_FFI_SHIM_H
#define TYPEFLOW_FFI_SHIM_H

#include <string.h>

#include "../../../crates/typeflow-ffi/include/typeflow.h"

static inline uint32_t typeflow_ffi_context_secure_input(void) {
    return TF_CONTEXT_SECURE_INPUT;
}

static inline uint32_t typeflow_ffi_context_automatic_processing_disabled(void) {
    return TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED;
}

static inline uint32_t typeflow_ffi_context_automatic_switching_disabled(void) {
    return TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED;
}

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

static inline TfComposition typeflow_ffi_empty_composition(void) {
    TfComposition composition;
    memset(&composition, 0, sizeof(composition));
    return composition;
}

#endif
